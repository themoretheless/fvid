use fvid::media::{self, CopyOptions, SubtitleCodec, SubtitleConvertOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-convert-burn-budget-parity")
}

fn fvid_bin() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn convert_subtitles_enforces_matroska_container_packet_bytes_and_cli_memory_budget_refusal() {
    let dir = fixture_dir();
    let srt = dir.join("sample.srt");
    let ass = dir.join("sample.ass");
    let muxed_mkv = dir.join("muxed-audio-and-subrip.mkv");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch244-sub-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. muxed-audio-and-subrip.mkv has a 128-byte audio packet on track 1 and a 16-byte SubRip cue on track 2
    // (formatted ASS payload is "0,0,Default,,0,0,0,,Matroska SubRip!" = 36 bytes).
    // With max_packet_bytes = 64, the subtitle cue fits in 64 bytes, but the Matroska container has a 128-byte packet!
    // Both plan_convert_subtitles_with_controls and convert_subtitles_with_controls must reject the oversized container packet.
    let sub_opts = SubtitleConvertOptions {
        streams: vec![],
        codec: SubtitleCodec::Ass,
    };
    let mut pkt64 = CopyOptions::default();
    pkt64.max_packet_bytes = 64;
    let plan_err = media::plan_convert_subtitles_with_controls(&muxed_mkv, &sub_opts, &pkt64)
        .unwrap_err();
    assert!(
        plan_err.contains("exceeds"),
        "plan_convert_subtitles_with_controls pkt64 returned: {plan_err}"
    );
    let out_mkv = temp_dir.join("muxed-out.mkv");
    let exec_err =
        media::convert_subtitles_with_controls(&muxed_mkv, &out_mkv, &sub_opts, &pkt64)
            .unwrap_err();
    assert!(
        exec_err.contains("exceeds"),
        "convert_subtitles_with_controls pkt64 returned: {exec_err}"
    );
    assert!(!out_mkv.exists());

    // With default limits, muxed_mkv converts cleanly
    let ok_plan =
        media::plan_convert_subtitles_with_controls(&muxed_mkv, &sub_opts, &CopyOptions::default())
            .unwrap();
    assert_eq!(ok_plan.command, "convert-subtitles");
    let stats = media::convert_subtitles_with_controls(
        &muxed_mkv,
        &out_mkv,
        &sub_opts,
        &CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(stats.cues, 1);
    let _ = std::fs::remove_file(&out_mkv);

    // 2. CLI `plan convert-subtitles` and `convert-subtitles` with `--max-memory-mib` return the owned
    // controlled-memory refusal ("convert-subtitles does not support controlled memory budgets")
    // rather than "unsupported convert-subtitles option: --max-memory-mib" or duplicate option mismatch.
    for src in [&srt, &ass, &muxed_mkv] {
        let plan_out = Command::new(fvid_bin())
            .args([
                "media",
                "plan",
                "convert-subtitles",
                src.to_str().unwrap(),
                "--max-memory-mib",
                "16",
            ])
            .output()
            .unwrap();
        assert!(!plan_out.status.success());
        let plan_stderr = String::from_utf8_lossy(&plan_out.stderr);
        assert!(
            plan_stderr.contains("convert-subtitles does not support controlled memory budgets"),
            "CLI plan convert-subtitles --max-memory-mib stderr for {}: {plan_stderr}",
            src.display()
        );

        let cli_out_mkv = temp_dir.join("cli-mem.mkv");
        let exec_out = Command::new(fvid_bin())
            .args([
                "media",
                "convert-subtitles",
                src.to_str().unwrap(),
                cli_out_mkv.to_str().unwrap(),
                "--max-memory-mib",
                "16",
            ])
            .output()
            .unwrap();
        assert!(!exec_out.status.success());
        let exec_stderr = String::from_utf8_lossy(&exec_out.stderr);
        assert!(
            exec_stderr.contains("convert-subtitles does not support controlled memory budgets"),
            "CLI convert-subtitles --max-memory-mib stderr for {}: {exec_stderr}",
            src.display()
        );
        assert!(!cli_out_mkv.exists());
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn burn_subtitles_enforces_input_packet_bytes_and_matches_plan_and_execution_errors() {
    let dir = fixture_dir();
    let srt = dir.join("sample.srt");
    let y4m = dir.join("burn-primary.y4m");
    let mkv_video = dir.join("burn-source-ffv1.mkv");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch244-burn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. On Y4M input (96x64 YUV420 = 9216 bytes per frame, encoded FFV1 packet ~50 bytes),
    // when max_packet_bytes = 4096 (smaller than raw Y4M frame 9216 bytes, but larger than encoded FFV1 packet),
    // both plan_burn_subtitles and burn_subtitles must reject with "Y4M frame byte length exceeds packet byte limit".
    let mut pkt4096 = CopyOptions::default();
    pkt4096.max_packet_bytes = 4096;
    let y4m_plan_err = media::plan_burn_subtitles(&y4m, &srt, &pkt4096).unwrap_err();
    assert!(
        y4m_plan_err.contains("Y4M frame byte length exceeds packet byte limit"),
        "plan_burn_subtitles y4m pkt4096 returned: {y4m_plan_err}"
    );
    let out_y4m_burn = temp_dir.join("y4m-burn-4096.mkv");
    let y4m_exec_err = media::burn_subtitles(&y4m, &out_y4m_burn, &srt, &pkt4096).unwrap_err();
    assert!(
        y4m_exec_err.contains("Y4M frame byte length exceeds packet byte limit"),
        "burn_subtitles y4m pkt4096 returned: {y4m_exec_err}"
    );
    assert!(!out_y4m_burn.exists());

    // 2. On Matroska FFV1 input, when max_packet_bytes = 2 (smaller than input FFV1 packets),
    // both plan_burn_subtitles and burn_subtitles must reject the oversized input packet during Matroska reader open/scan.
    let mut pkt2 = CopyOptions::default();
    pkt2.max_packet_bytes = 2;
    let mkv_plan_err = media::plan_burn_subtitles(&mkv_video, &srt, &pkt2).unwrap_err();
    assert!(
        mkv_plan_err.contains("exceeds"),
        "plan_burn_subtitles mkv pkt2 returned: {mkv_plan_err}"
    );
    let out_mkv_burn = temp_dir.join("mkv-burn-2.mkv");
    let mkv_exec_err =
        media::burn_subtitles(&mkv_video, &out_mkv_burn, &srt, &pkt2).unwrap_err();
    assert!(
        mkv_exec_err.contains("exceeds"),
        "burn_subtitles mkv pkt2 returned: {mkv_exec_err}"
    );
    assert!(!out_mkv_burn.exists());

    // 3. When max_packets = Some(0), both plan_burn_subtitles and burn_subtitles reject before publishing.
    let mut zero_pkt = CopyOptions::default();
    zero_pkt.max_packets = Some(0);
    for (name, src) in [("y4m", &y4m), ("mkv", &mkv_video)] {
        let p_err = media::plan_burn_subtitles(src, &srt, &zero_pkt).unwrap_err();
        assert!(
            p_err.contains("packet count exceeds limit"),
            "plan_burn_subtitles zero_pkt for {name} returned: {p_err}"
        );
        let out_zero = temp_dir.join(format!("zero-{name}.mkv"));
        let e_err = media::burn_subtitles(src, &out_zero, &srt, &zero_pkt).unwrap_err();
        assert!(
            e_err.contains("packet count exceeds limit"),
            "burn_subtitles zero_pkt for {name} returned: {e_err}"
        );
        assert!(!out_zero.exists());
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn cli_burn_subtitles_xfade_and_transcode_lossless_preserve_specific_policy_refusals() {
    let dir = fixture_dir();
    let srt = dir.join("sample.srt");
    let y4m_a = dir.join("burn-primary.y4m");
    let y4m_b = dir.join("burn-secondary.y4m");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch244-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. CLI `burn-subtitles` and `plan burn-subtitles` with `--streams 1` or `--max-memory-mib 16`
    // return the specific owned policy error instead of falling through to generic CLI help.
    for flag_pair in [["--streams", "1"], ["--max-memory-mib", "16"]] {
        let plan_out = Command::new(fvid_bin())
            .args([
                "media",
                "plan",
                "burn-subtitles",
                y4m_a.to_str().unwrap(),
                "--subs",
                srt.to_str().unwrap(),
                flag_pair[0],
                flag_pair[1],
            ])
            .output()
            .unwrap();
        assert!(!plan_out.status.success());
        let plan_stderr = String::from_utf8_lossy(&plan_out.stderr);
        assert!(
            plan_stderr.contains(
                "owned subtitle burn-in does not implement requested metadata/memory policy"
            ),
            "plan burn-subtitles {:?} stderr: {plan_stderr}",
            flag_pair
        );

        let out_burn = temp_dir.join("cli-burn-policy.mkv");
        let exec_out = Command::new(fvid_bin())
            .args([
                "media",
                "burn-subtitles",
                y4m_a.to_str().unwrap(),
                out_burn.to_str().unwrap(),
                "--subs",
                srt.to_str().unwrap(),
                flag_pair[0],
                flag_pair[1],
            ])
            .output()
            .unwrap();
        assert!(!exec_out.status.success());
        let exec_stderr = String::from_utf8_lossy(&exec_out.stderr);
        assert!(
            exec_stderr.contains(
                "owned subtitle burn-in does not implement requested metadata/memory policy"
            ),
            "burn-subtitles {:?} stderr: {exec_stderr}",
            flag_pair
        );
        assert!(!out_burn.exists());
    }

    // 2. CLI `xfade` and `plan xfade` with `--max-memory-mib 16` or `--streams 1` return the specific owned refusal
    for flag_pair in [["--streams", "1"], ["--max-memory-mib", "16"]] {
        let plan_out = Command::new(fvid_bin())
            .args([
                "media",
                "plan",
                "xfade",
                y4m_a.to_str().unwrap(),
                y4m_b.to_str().unwrap(),
                "--xfade-duration",
                "0.5",
                flag_pair[0],
                flag_pair[1],
            ])
            .output()
            .unwrap();
        assert!(!plan_out.status.success());
        let plan_stderr = String::from_utf8_lossy(&plan_out.stderr);
        assert!(
            plan_stderr.contains("cross-fade plan is not supported by the owned backend"),
            "plan xfade {:?} stderr: {plan_stderr}",
            flag_pair
        );

        let out_xfade = temp_dir.join("cli-xfade-policy.mkv");
        let exec_out = Command::new(fvid_bin())
            .args([
                "media",
                "xfade",
                y4m_a.to_str().unwrap(),
                y4m_b.to_str().unwrap(),
                out_xfade.to_str().unwrap(),
                "--xfade-duration",
                "0.5",
                flag_pair[0],
                flag_pair[1],
            ])
            .output()
            .unwrap();
        assert!(!exec_out.status.success());
        let exec_stderr = String::from_utf8_lossy(&exec_out.stderr);
        assert!(
            exec_stderr.contains("cross-fade request is not supported by the owned backend"),
            "xfade {:?} stderr: {exec_stderr}",
            flag_pair
        );
        assert!(!out_xfade.exists());
    }

    // 3. CLI `xfade` and `plan xfade` with `--max-packet-bytes 4096` (smaller than 9216-byte Y4M frame)
    // both reject with "Y4M frame byte length exceeds packet byte limit".
    let xfade_plan_pkt = Command::new(fvid_bin())
        .args([
            "media",
            "plan",
            "xfade",
            y4m_a.to_str().unwrap(),
            y4m_b.to_str().unwrap(),
            "--xfade-duration",
            "0.5",
            "--max-packet-bytes",
            "4096",
        ])
        .output()
        .unwrap();
    assert!(!xfade_plan_pkt.status.success());
    assert!(
        String::from_utf8_lossy(&xfade_plan_pkt.stderr)
            .contains("Y4M frame byte length exceeds packet byte limit")
    );
    let out_xfade_pkt = temp_dir.join("cli-xfade-pkt.mkv");
    let xfade_exec_pkt = Command::new(fvid_bin())
        .args([
            "media",
            "xfade",
            y4m_a.to_str().unwrap(),
            y4m_b.to_str().unwrap(),
            out_xfade_pkt.to_str().unwrap(),
            "--xfade-duration",
            "0.5",
            "--max-packet-bytes",
            "4096",
        ])
        .output()
        .unwrap();
    assert!(!xfade_exec_pkt.status.success());
    assert!(
        String::from_utf8_lossy(&xfade_exec_pkt.stderr)
            .contains("Y4M frame byte length exceeds packet byte limit")
    );
    assert!(!out_xfade_pkt.exists());

    let _ = std::fs::remove_dir_all(&temp_dir);
}
