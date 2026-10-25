//! Regression tests for Batch 247 (`FVID-0254`):
//! 1. `fvid_media::owned_mp4_remux::plan_remux`, `fvid_media::owned_mp4_remux::remux`, and
//!    `fvid::media::plan_remux` / `fvid::media::remux` enforce `options.max_packet_bytes` on
//!    MP4 sample sizes during planning (rejecting packets larger than `max_packet_bytes` with
//!    `"MP4 packet exceeds budget"`), and reject `options.max_packets == Some(0)` with
//!    `"Matroska output requires at least one packet"`.
//! 2. `fvid::container::mp4_concat::open_with_packet_limit`, `fvid::native_plan::concat_mp4_matroska_with_options`,
//!    `fvid::native_export::try_concat_mp4_matroska_with_options`, and `fvid::media::plan_concat` /
//!    `fvid::media::concat` enforce `options.max_packet_bytes` on MP4 sample sizes across all segments
//!    (rejecting oversized MP4 samples with `"MP4 packet exceeds budget"` instead of planning success
//!    or silently falling through to audio decode concat), and `write_with_options` rejects
//!    `options.max_packets == Some(0)` with `"Matroska output requires at least one packet"`.
//! 3. Single-stream WAVE and ADTS operations (`fvid::media::trim_pcm`, `fvid::media::plan_trim_pcm`,
//!    `fvid::media::concat`, `fvid::media::plan_concat`, `fvid::media::remux`, `fvid::media::plan_remux`)
//!    reject multi-stream selections containing non-zero stream indices such as `options.streams = vec![0, 1]`
//!    with canonical `"WAVE has only stream 0"` / `"ADTS has only stream 0"` diagnostics, and
//!    `fvid::native_plan::remux_with_options` enforces positive `max_packet_bytes` and `check_rss_budget`.

use std::path::PathBuf;

fn y4m_budget_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-export-budget-parity")
}

fn write_synthetic_wave(path: &std::path::Path) {
    let pcm: [i16; 4] = [256, -256, 512, -512];
    let mut pcm_bytes = Vec::with_capacity(8);
    for sample in pcm {
        pcm_bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let mut fmt = Vec::with_capacity(16);
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&48_000u32.to_le_bytes());
    fmt.extend_from_slice(&96_000u32.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());

    let mut riff_body = Vec::new();
    riff_body.extend_from_slice(b"WAVEfmt ");
    riff_body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    riff_body.extend_from_slice(&fmt);
    riff_body.extend_from_slice(b"data");
    riff_body.extend_from_slice(&(pcm_bytes.len() as u32).to_le_bytes());
    riff_body.extend_from_slice(&pcm_bytes);

    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(riff_body.len() as u32).to_le_bytes());
    file.extend_from_slice(&riff_body);
    std::fs::write(path, file).unwrap();
}

fn write_synthetic_adts(path: &std::path::Path) {
    let payload: [u8; 6] = [0x21, 0x10, 0x04, 0x60, 0x8C, 0x1C];
    let frame_len = 7 + payload.len();
    let header: [u8; 7] = [
        0xFF,
        0xF1,
        (1 << 6) | (3 << 2) | ((1 >> 2) & 0x01),
        ((1 & 0x03) << 6) | (((frame_len >> 11) & 0x03) as u8),
        ((frame_len >> 3) & 0xFF) as u8,
        (((frame_len & 0x07) << 5) as u8) | 0x1F,
        0xFC,
    ];
    let mut bytes = Vec::with_capacity(frame_len);
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&payload);
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn mp4_matroska_remux_plan_and_exec_enforce_packet_bytes_and_nonzero_packets() {
    let mp4 = y4m_budget_fixture_dir().join("avc-32x16.mp4");
    let dir = std::env::temp_dir().join(format!(
        "fvid-mp4-mkv-remux-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Each AVC I_PCM packet in avc-32x16.mp4 is 778 bytes.
    // With max_packet_bytes = 500, both plan_remux and remux must reject the 778-byte sample.
    let mut tight_pkt = fvid::media::CopyOptions::default();
    tight_pkt.max_packet_bytes = 500;

    let plan_err = fvid::media::plan_remux(&mp4, &tight_pkt).unwrap_err();
    assert!(
        plan_err.contains("MP4 packet exceeds budget"),
        "plan_remux on MP4 must reject 778-byte packet when max_packet_bytes=500, got: {plan_err}"
    );

    let owned_plan_err = fvid_media::owned_mp4_remux::plan_remux(&mp4, &tight_pkt).unwrap_err();
    assert!(
        owned_plan_err.contains("MP4 packet exceeds budget"),
        "owned_mp4_remux::plan_remux must reject 778-byte packet when max_packet_bytes=500, got: {owned_plan_err}"
    );

    let out_tight = dir.join("tight.mkv");
    let exec_err = fvid::media::remux(&mp4, &out_tight, &tight_pkt).unwrap_err();
    assert!(
        exec_err.contains("MP4 packet exceeds budget"),
        "remux on MP4 must reject 778-byte packet when max_packet_bytes=500, got: {exec_err}"
    );
    assert!(!out_tight.exists());

    // With max_packets = Some(0), both plan_remux and remux must refuse empty Matroska output.
    let mut zero_pkts = fvid::media::CopyOptions::default();
    zero_pkts.max_packets = Some(0);

    let zero_plan_err = fvid::media::plan_remux(&mp4, &zero_pkts).unwrap_err();
    assert!(
        zero_plan_err.contains("Matroska output requires at least one packet"),
        "plan_remux on MP4 must reject max_packets=Some(0), got: {zero_plan_err}"
    );

    let out_zero = dir.join("zero.mkv");
    let zero_exec_err = fvid::media::remux(&mp4, &out_zero, &zero_pkts).unwrap_err();
    assert!(
        zero_exec_err.contains("Matroska output requires at least one packet"),
        "remux on MP4 must reject max_packets=Some(0), got: {zero_exec_err}"
    );
    assert!(!out_zero.exists());

    // With max_packet_bytes = 800 and max_packets = Some(1), both plan_remux and remux succeed.
    let mut valid_opts = fvid::media::CopyOptions::default();
    valid_opts.max_packet_bytes = 800;
    valid_opts.max_packets = Some(1);

    let plan = fvid::media::plan_remux(&mp4, &valid_opts)
        .expect("plan_remux with max_packet_bytes=800, max_packets=1 should succeed");
    assert!(
        plan.steps
            .iter()
            .any(|s| s.detail.contains("copy 1 compressed packets, 780 payload bytes")),
        "unexpected plan steps: {:?}",
        plan.steps
    );

    let out_ok = dir.join("ok.mkv");
    let stats = fvid::media::remux(&mp4, &out_ok, &valid_opts)
        .expect("remux with max_packet_bytes=800, max_packets=1 should succeed");
    assert_eq!(stats.packets, 1);
    assert_eq!(stats.payload_bytes, 780);
    assert!(out_ok.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mp4_matroska_concat_enforces_packet_bytes_and_nonzero_packets_in_plan_and_execution() {
    let mp4 = y4m_budget_fixture_dir().join("avc-32x16.mp4");
    let sources = vec![mp4.clone(), mp4.clone()];
    let dir = std::env::temp_dir().join(format!(
        "fvid-mp4-mkv-concat-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 1. Tight packet budget (500 bytes < 778-byte AVC sample) must be rejected by
    // open_with_packet_limit, concat_mp4_matroska_with_options, plan_concat, and concat.
    let open_err = fvid::container::mp4_concat::open_with_packet_limit(&sources, None, 500)
        .err()
        .expect("open_with_packet_limit should reject 778-byte sample when limit=500");
    assert!(
        open_err.to_string().contains("MP4 packet exceeds budget"),
        "expected MP4 packet exceeds budget from open_with_packet_limit, got: {open_err}"
    );

    let mut tight_pkt = fvid::media::CopyOptions::default();
    tight_pkt.max_packet_bytes = 500;

    let native_plan_err =
        fvid::native_plan::concat_mp4_matroska_with_options(&sources, &tight_pkt).unwrap_err();
    assert!(
        native_plan_err.contains("MP4 packet exceeds budget"),
        "concat_mp4_matroska_with_options must reject 778-byte sample when max_packet_bytes=500, got: {native_plan_err}"
    );

    let media_plan_err = fvid::media::plan_concat(&sources, &tight_pkt).unwrap_err();
    assert!(
        media_plan_err.contains("MP4 packet exceeds budget"),
        "media::plan_concat must reject 778-byte sample when max_packet_bytes=500, got: {media_plan_err}"
    );

    let out_tight = dir.join("concat-tight.mkv");
    let concat_err = fvid::media::concat(&sources, &out_tight, &tight_pkt).unwrap_err();
    assert!(
        concat_err.contains("MP4 packet exceeds budget"),
        "media::concat must reject 778-byte sample when max_packet_bytes=500, got: {concat_err}"
    );
    assert!(!out_tight.exists());

    // 2. max_packets = Some(0) must be rejected in write_with_options and try_concat_mp4_matroska_with_options
    let mut zero_pkts = fvid::media::CopyOptions::default();
    zero_pkts.max_packets = Some(0);

    let mut segments = fvid::container::mp4_concat::open_with_packet_limit(&sources, None, 800)
        .unwrap()
        .expect("segments should open when limit=800");
    let mut cursor = std::io::Cursor::new(Vec::new());
    let write_zero_err =
        fvid::container::mp4_concat::write_with_options(&mut segments, &mut cursor, &zero_pkts)
            .unwrap_err();
    assert!(
        write_zero_err
            .to_string()
            .contains("Matroska output requires at least one packet")
            || write_zero_err
                .to_string()
                .contains("packet work limit requires at least one packet"),
        "write_with_options must reject max_packets=Some(0), got: {write_zero_err}"
    );

    let out_zero = dir.join("concat-zero.mkv");
    let exec_zero_err = fvid::media::concat(&sources, &out_zero, &zero_pkts).unwrap_err();
    assert!(
        exec_zero_err.contains("requires at least one packet"),
        "media::concat on MP4 with max_packets=Some(0) must fail cleanly, got: {exec_zero_err}"
    );
    assert!(!out_zero.exists());

    // 3. Positive budget (800 bytes >= 778-byte sample) and max_packets = Some(3) succeeds.
    let mut ok_opts = fvid::media::CopyOptions::default();
    ok_opts.max_packet_bytes = 800;
    ok_opts.max_packets = Some(3);

    let plan = fvid::media::plan_concat(&sources, &ok_opts)
        .expect("plan_concat with max_packet_bytes=800, max_packets=3 should succeed");
    assert_eq!(plan.command, "concat");

    let out_ok = dir.join("concat-ok.mkv");
    let stats = fvid::media::concat(&sources, &out_ok, &ok_opts)
        .expect("concat with max_packet_bytes=800, max_packets=3 should succeed");
    assert_eq!(stats.packets, 3);
    assert_eq!(stats.payload_bytes, 3 * 780);
    assert!(out_ok.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn single_stream_wave_and_adts_reject_multi_index_streams_and_native_plan_remux_enforces_limits() {
    let mp4 = y4m_budget_fixture_dir().join("avc-32x16.mp4");
    let mkv = y4m_budget_fixture_dir().join("avc-32x16.mkv");
    let dir = std::env::temp_dir().join(format!(
        "fvid-single-stream-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let wav = dir.join("pcm.wav");
    let adts = dir.join("aac.aac");
    write_synthetic_wave(&wav);
    write_synthetic_adts(&adts);

    // 1. Multi-stream selection [0, 1] starts with 0, so `.first()` was `Some(&0)`,
    // bypassing the single-stream check and producing non-canonical errors.
    let mut multi_streams = fvid::media::CopyOptions::default();
    multi_streams.streams = vec![0, 1];

    let out_wav_trim = dir.join("trim.wav");
    let err = fvid::media::trim_pcm(&wav, &out_wav_trim, 0, 50_000, &multi_streams).unwrap_err();
    assert_eq!(err, "WAVE has only stream 0");

    let err = fvid::media::plan_trim_pcm(&wav, 0, 50_000, &multi_streams).unwrap_err();
    assert_eq!(err, "WAVE has only stream 0");

    let out_wav_concat = dir.join("concat.wav");
    let err = fvid::media::concat(
        &[wav.clone(), wav.clone()],
        &out_wav_concat,
        &multi_streams,
    )
    .unwrap_err();
    assert_eq!(err, "WAVE has only stream 0");

    let err = fvid::media::plan_concat(&[wav.clone(), wav.clone()], &multi_streams).unwrap_err();
    assert_eq!(err, "WAVE has only stream 0");

    let out_adts_remux = dir.join("remux.mka");
    let err = fvid::media::remux(&adts, &out_adts_remux, &multi_streams).unwrap_err();
    assert_eq!(err, "ADTS has only stream 0");

    let err = fvid::media::plan_remux(&adts, &multi_streams).unwrap_err();
    assert_eq!(err, "ADTS has only stream 0");

    let out_adts_concat = dir.join("concat.mka");
    let err = fvid::media::concat(
        &[adts.clone(), adts.clone()],
        &out_adts_concat,
        &multi_streams,
    )
    .unwrap_err();
    assert_eq!(err, "ADTS has only stream 0");

    let err = fvid::media::plan_concat(&[adts.clone(), adts.clone()], &multi_streams).unwrap_err();
    assert_eq!(err, "ADTS has only stream 0");

    // 2. native_plan::remux_with_options must enforce max_packet_bytes > 0 and check_rss_budget
    let mut zero_pkt_bytes = fvid::media::CopyOptions::default();
    zero_pkt_bytes.max_packet_bytes = 0;
    for source in [&mp4, &mkv, &adts] {
        let err = fvid::native_plan::remux_with_options(source, &zero_pkt_bytes).unwrap_err();
        assert!(
            err.contains("packet byte limit must be positive"),
            "native_plan::remux_with_options should reject max_packet_bytes=0 on {}, got: {err}",
            source.display()
        );
    }

    let mut tight_rss = fvid::media::CopyOptions::default();
    tight_rss.max_rss_bytes = Some(1);
    for source in [&mp4, &mkv, &adts] {
        let err = fvid::native_plan::remux_with_options(source, &tight_rss).unwrap_err();
        assert!(
            err.contains("rss budget exceeded"),
            "native_plan::remux_with_options should reject tight RSS budget on {}, got: {err}",
            source.display()
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
