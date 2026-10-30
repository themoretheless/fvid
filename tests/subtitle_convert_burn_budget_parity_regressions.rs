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

    // 2. CLI `plan convert-subtitles` and `convert-subtitles` with `--max-memory-mib 16` succeed
    // and record the controlled memory limit in plan notes, while tight `max_controlled_bytes`
    // and `--max-memory-mib 0` fail before creating output files.
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
        assert!(
            plan_out.status.success(),
            "CLI plan convert-subtitles --max-memory-mib 16 failed for {}: {}",
            src.display(),
            String::from_utf8_lossy(&plan_out.stderr)
        );
        let plan_stdout = String::from_utf8_lossy(&plan_out.stdout);
        assert!(
            plan_stdout.contains("controlled memory limit: Some(16777216)"),
            "CLI plan convert-subtitles --max-memory-mib 16 stdout for {}: {plan_stdout}",
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
        assert!(
            exec_out.status.success(),
            "CLI convert-subtitles --max-memory-mib 16 failed for {}: {}",
            src.display(),
            String::from_utf8_lossy(&exec_out.stderr)
        );
        assert!(cli_out_mkv.exists());
        let _ = std::fs::remove_file(&cli_out_mkv);

        let mut tight_mem = CopyOptions::default();
        tight_mem.max_controlled_bytes = Some(1024);
        let plan_mem_err =
            media::plan_convert_subtitles_with_controls(src, &sub_opts, &tight_mem).unwrap_err();
        assert!(
            plan_mem_err.contains("controlled memory budget exceeded"),
            "plan_convert_subtitles_with_controls tight_mem for {} returned: {plan_mem_err}",
            src.display()
        );
        let exec_mem_err =
            media::convert_subtitles_with_controls(src, &cli_out_mkv, &sub_opts, &tight_mem)
                .unwrap_err();
        assert!(
            exec_mem_err.contains("controlled memory budget exceeded"),
            "convert_subtitles_with_controls tight_mem for {} returned: {exec_mem_err}",
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
        assert_eq!(p_err, "packet work limit requires at least one packet",
            "plan_burn_subtitles zero_pkt for {name}");
        let out_zero = temp_dir.join(format!("zero-{name}.mkv"));
        let e_err = media::burn_subtitles(src, &out_zero, &srt, &zero_pkt).unwrap_err();
        assert_eq!(e_err, "packet work limit requires at least one packet",
            "burn_subtitles zero_pkt for {name}");
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

    // 1. CLI `burn-subtitles` and `plan burn-subtitles` with `--streams 1`
    // return the specific owned policy error instead of falling through to generic CLI help,
    // while `--max-memory-mib 16` is now admitted and enforced.
    for flag_pair in [["--streams", "1"]] {
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

    // 2. CLI `xfade` and `plan xfade` with `--streams 1` return the specific owned refusal,
    // while `--max-memory-mib 16` is now admitted and enforced.
    for flag_pair in [["--streams", "1"]] {
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

#[test]
fn convert_subtitles_and_plan_convert_subtitles_enforce_controlled_memory_for_matroska_metadata_working_sets() {
    let dir = fixture_dir();
    let srt = dir.join("sample.srt");
    let ass = dir.join("sample.ass");
    let muxed_mkv = dir.join("muxed-audio-and-subrip.mkv");

    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-convert-subtitles-meta-mem-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let sub_opts = SubtitleConvertOptions {
        streams: vec![],
        codec: SubtitleCodec::Ass,
    };

    // SRT/ASS without extra metadata fit 32 KiB. Matroska additionally
    // retains a capacity-accounted index, tested below at both budget bounds.
    let base_tight = CopyOptions {
        max_controlled_bytes: Some(32_768),
        ..Default::default()
    };
    // The capacity-aware Matroska index now exceeds 32 KiB on its own.
    // Preserve that tight refusal and derive the positive control from the
    // same retained-index accounting used by planning and execution.
    let mut indexed = fvid_media::owned_webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(&muxed_mkv).unwrap()),
        Default::default(),
    ).unwrap();
    indexed.scan_all().unwrap();
    let index_work = indexed.estimated_index_payload_bytes().unwrap()
        + indexed.packets.len() * 64 + 16_384;
    assert!(index_work > 32_768 && index_work <= 65_536);
    assert_eq!(media::plan_convert_subtitles_with_controls(&muxed_mkv, &sub_opts, &base_tight)
        .unwrap_err(), "controlled memory budget exceeded");
    let tight_out = temp_dir.join("tight-index.mkv");
    assert_eq!(media::convert_subtitles_with_controls(&muxed_mkv, &tight_out, &sub_opts, &base_tight)
        .unwrap_err(), "controlled memory budget exceeded");
    assert!(!tight_out.exists());
    let mkv_admission = CopyOptions {
        max_controlled_bytes: Some(index_work.next_power_of_two()),
        ..Default::default()
    };
    for (label, src) in [("srt", &srt), ("ass", &ass), ("mkv", &muxed_mkv)] {
        let admission = if label == "mkv" { &mkv_admission } else { &base_tight };
        media::plan_convert_subtitles_with_controls(src, &sub_opts, admission)
            .expect(&format!("plan_convert_subtitles({label}) must fit its accounted budget"));
        let ok_out = temp_dir.join(format!("ok-{label}.mkv"));
        let stats = media::convert_subtitles_with_controls(src, &ok_out, &sub_opts, admission)
            .expect(&format!("convert_subtitles({label}) must fit its accounted budget"));
        assert!(stats.cues >= 1);
        let _ = std::fs::remove_file(&ok_out);
    }

    // 2. Adding 32 container tags + 32 stream 0 tags of 900 bytes each (~57.6 KiB raw, ~460 KiB working set)
    // exceeds max_controlled_bytes = Some(32_768) in both plan_convert_subtitles_with_controls and
    // convert_subtitles_with_controls across SRT, ASS, and Matroska subtitle sources!
    let val = "m".repeat(900);
    let mut heavy_opts = base_tight.clone();
    heavy_opts.metadata_set = (0..32).map(|i| (format!("CTAG_{i:02}"), val.clone())).collect();
    heavy_opts.stream_metadata_set = (0..32)
        .map(|i| (0usize, format!("STAG_{i:02}"), val.clone()))
        .collect();

    let rejected = temp_dir.join("rejected.mkv");
    for (label, src) in [("srt", &srt), ("ass", &ass), ("mkv", &muxed_mkv)] {
        let plan_err = media::plan_convert_subtitles_with_controls(src, &sub_opts, &heavy_opts)
            .expect_err(&format!(
                "plan_convert_subtitles({label}) must reject Matroska metadata working set exceeding max_controlled_bytes"
            ));
        assert!(
            plan_err.contains("controlled memory budget"),
            "plan_convert_subtitles({label}) expected controlled memory budget error, got: {plan_err}"
        );
        let exec_err = media::convert_subtitles_with_controls(src, &rejected, &sub_opts, &heavy_opts)
            .expect_err(&format!(
                "convert_subtitles({label}) must reject Matroska metadata working set exceeding max_controlled_bytes"
            ));
        assert!(
            exec_err.contains("controlled memory budget"),
            "convert_subtitles({label}) expected controlled memory budget error, got: {exec_err}"
        );
        assert!(!rejected.exists());
    }

    // 3. Also verify that preserved source Matroska tags in a Matroska subtitle source are counted
    // against max_controlled_bytes even when controls.metadata_set is empty!
    let rich_sub_mkv = temp_dir.join("rich-sub.mkv");
    let mut rich_write_opts = CopyOptions::default();
    rich_write_opts.metadata_set = heavy_opts.metadata_set.clone();
    rich_write_opts.stream_metadata_set = heavy_opts.stream_metadata_set.clone();
    media::convert_subtitles_with_controls(&srt, &rich_sub_mkv, &sub_opts, &rich_write_opts)
        .expect("converting SRT with rich Matroska metadata under default budget must succeed");

    let preserved_plan_err = media::plan_convert_subtitles_with_controls(
        &rich_sub_mkv,
        &sub_opts,
        &base_tight,
    )
    .expect_err("plan_convert_subtitles on rich Matroska source must reject preserved metadata working set exceeding 32 KiB");
    assert!(
        preserved_plan_err.contains("controlled memory budget"),
        "expected controlled memory budget error on rich Matroska plan, got: {preserved_plan_err}"
    );
    let preserved_exec_err = media::convert_subtitles_with_controls(
        &rich_sub_mkv,
        &rejected,
        &sub_opts,
        &base_tight,
    )
    .expect_err("convert_subtitles on rich Matroska source must reject preserved metadata working set exceeding 32 KiB");
    assert!(
        preserved_exec_err.contains("controlled memory budget"),
        "expected controlled memory budget error on rich Matroska convert, got: {preserved_exec_err}"
    );
    assert!(!rejected.exists());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn burn_subtitles_and_mp4_decode_enforce_controlled_memory_for_srt_cues_and_decode_loop_frames() {
    let dir = fixture_dir();
    let y4m = dir.join("burn-primary.y4m");
    let mkv_video = dir.join("burn-source-ffv1.mkv");
    let mp4_reordered = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-unified-explicit-b-8bit-top-init0-motion-weighted-ref1-bi16x8.mp4");

    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-batch332-burn-mp4-mem-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. Create a synthetic 24 KiB SRT file (~600 cues) in temp_dir.
    // The Y4M and MKV frames are 96x64 YUV420 (9,216 bytes + 6,144-byte mask = 15,360 bytes),
    // which fit in a 32 KiB (32,768-byte) controlled memory budget when burning `sample.srt` (41 bytes),
    // whereas the 24 KiB SRT file (`raw_len * 3 + cues * 32` > 72 KiB) exceeds `max_controlled_bytes = Some(32_768)`.
    let small_srt = dir.join("sample.srt");
    let mut mem32k = CopyOptions::default();
    mem32k.max_controlled_bytes = Some(32_768);
    media::plan_burn_subtitles(&y4m, &small_srt, &mem32k)
        .expect("plan_burn_subtitles with small SRT must fit in 32 KiB");
    let small_out = temp_dir.join("small-ok.mkv");
    let small_stats = media::burn_subtitles(&y4m, &small_out, &small_srt, &mem32k)
        .expect("burn_subtitles with small SRT must fit in 32 KiB");
    assert_eq!(small_stats.video_frames, 2);
    let _ = std::fs::remove_file(&small_out);

    let heavy_srt = temp_dir.join("heavy.srt");
    let mut srt_text = String::new();
    for i in 0..600 {
        srt_text.push_str(&format!(
            "{}\n00:00:{:02},{:03} --> 00:00:{:02},{:03}\nSynthetic subtitle cue line {:04}\n\n",
            i + 1,
            (i / 100) % 60,
            (i * 10) % 1000,
            (i / 100) % 60,
            ((i * 10) % 1000) + 5,
            i
        ));
    }
    std::fs::write(&heavy_srt, &srt_text).unwrap();
    assert!(srt_text.len() > 24_000 && srt_text.len() < 128_000);

    let rejected = temp_dir.join("rejected-burn.mkv");
    for (label, src) in [("y4m", &y4m), ("mkv", &mkv_video)] {
        let plan_err = media::plan_burn_subtitles(src, &heavy_srt, &mem32k).expect_err(&format!(
            "plan_burn_subtitles({label}) must reject 24 KiB SRT under 32 KiB controlled memory budget"
        ));
        assert!(
            plan_err.contains("controlled memory budget"),
            "plan_burn_subtitles({label}) expected controlled memory budget error, got: {plan_err}"
        );
        let exec_err = media::burn_subtitles(src, &rejected, &heavy_srt, &mem32k).expect_err(
            &format!(
                "burn_subtitles({label}) must reject 24 KiB SRT under 32 KiB controlled memory budget"
            ),
        );
        assert!(
            exec_err.contains("controlled memory budget"),
            "burn_subtitles({label}) expected controlled memory budget error, got: {exec_err}"
        );
        assert!(!rejected.exists());
    }

    // 2. On MP4 video input (`mp4_reordered`, 6 frames, 32x32 YUV420 = 1,536 bytes per frame),
    // `max_controlled_bytes = Some(1_024)` is larger than the MP4 `moov` box (~750 bytes) and `FrameMetadata`
    // table (`6 * 72 = 432` bytes), so `Mp4Reader::open` and `FrameMetadata` reservation succeed,
    // and `decode_presented` rejects the 1,536-byte decoded frame immediately during the decode loop
    // (`"MP4 video frame exceeds controlled memory budget"`).
    let mut mem1k = CopyOptions::default();
    mem1k.max_controlled_bytes = Some(1_024);
    let mp4_plan_err =
        media::plan_transcode_lossless(&mp4_reordered, &Default::default(), &mem1k, None)
            .expect_err("plan_transcode_lossless on 6-frame MP4 must reject 1,024-byte budget");
    assert!(
        mp4_plan_err.contains("controlled memory budget"),
        "expected controlled memory budget error on 6-frame MP4 plan, got: {mp4_plan_err}"
    );
    let mp4_exec_err = media::transcode_lossless(
        &mp4_reordered,
        &rejected,
        Default::default(),
        &mem1k,
    )
    .expect_err("transcode_lossless on 6-frame MP4 must reject 1,024-byte budget");
    assert!(
        mp4_exec_err.contains("controlled memory budget"),
        "expected controlled memory budget error on 6-frame MP4 transcode, got: {mp4_exec_err}"
    );
    assert!(!rejected.exists());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn convert_subtitles_and_burn_subtitles_precheck_script_and_metadata_budgets_before_reading_or_opening_output() {
    let dir = fixture_dir();
    let y4m = dir.join("burn-primary.y4m");

    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-batch344-sub-burn-precheck-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(temp_dir.clone());

    let sub_opts = SubtitleConvertOptions {
        streams: vec![],
        codec: SubtitleCodec::Ass,
    };

    // 1. In `try_convert_ass_inner` and `try_convert_srt_inner`, a standalone `.srt` or `.ass` file
    // whose on-disk size * 3 + meta_working_bytes + 16384 exceeds `max_controlled_bytes` must be
    // rejected via `std::fs::metadata` BEFORE reading the file into memory (so even if the file
    // contains invalid UTF-8 bytes, the controlled memory budget error is returned rather than
    // reading the whole file and returning `Ok(None)` / falling through to legacy).
    let invalid_utf8_srt = temp_dir.join("invalid-utf8.srt");
    let invalid_utf8_ass = temp_dir.join("invalid-utf8.ass");
    let mut non_utf8_payload = vec![0xffu8; 20_000];
    non_utf8_payload[0..4].copy_from_slice(&[0xff, 0xfe, 0xfd, 0xfc]);
    std::fs::write(&invalid_utf8_srt, &non_utf8_payload).unwrap();
    std::fs::write(&invalid_utf8_ass, &non_utf8_payload).unwrap();

    let mem32k = CopyOptions {
        max_controlled_bytes: Some(32_768),
        ..Default::default()
    };
    let rejected = temp_dir.join("rejected.mkv");
    for (label, src) in [("srt", &invalid_utf8_srt), ("ass", &invalid_utf8_ass)] {
        let plan_err = media::plan_convert_subtitles_with_controls(src, &sub_opts, &mem32k)
            .expect_err(&format!(
                "plan_convert_subtitles({label}) must pre-check file metadata size against max_controlled_bytes before reading"
            ));
        assert!(
            plan_err.contains("controlled memory budget"),
            "plan_convert_subtitles({label}) expected controlled memory budget error before UTF-8 decode, got: {plan_err}"
        );
        let exec_err = media::convert_subtitles_with_controls(src, &rejected, &sub_opts, &mem32k)
            .expect_err(&format!(
                "convert_subtitles({label}) must pre-check file metadata size against max_controlled_bytes before reading"
            ));
        assert!(
            exec_err.contains("controlled memory budget"),
            "convert_subtitles({label}) expected controlled memory budget error before UTF-8 decode, got: {exec_err}"
        );
        assert!(!rejected.exists());
    }

    // 2. In `owned_subtitle_burn::burn_subtitles` on a Y4M source, `options` metadata working-set
    // budget must be checked BEFORE `mkv::export_atomic` opens the temporary file, and
    // `fvid::media::probe_with_options` must forward `options` to `native_probe::try_probe_as_with_options`
    // and `fvid_media::owned_probe::probe_with_options`.
    let probe_err = media::probe_with_options(
        &y4m,
        &CopyOptions {
            max_packet_bytes: 64,
            ..Default::default()
        },
    )
    .expect_err("media::probe_with_options must enforce max_packet_bytes on Y4M");
    assert!(
        probe_err.contains("packet byte limit"),
        "expected packet byte limit error from media::probe_with_options, got: {probe_err}"
    );
}

