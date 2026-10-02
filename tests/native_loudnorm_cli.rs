use std::process::Command;
#[test]
fn loudnorm_cli_exports_long_wave_and_reports_prefix_progress_without_legacy() {
    let dir = std::env::temp_dir().join(format!("fvid-native-loudnorm-cli-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/loudnorm-dynamic.wav");
    let output = dir.join("long.wav");
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "loudnorm"])
        .arg(&fixture)
        .arg(&output)
        .args(["--max-memory-mib", "24"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(stats["backend"], "fvid dynamic loudnorm");
    assert_eq!(stats["sample_frames"], 576000);
    let prefix = dir.join("prefix.wav");
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "loudnorm"])
        .arg(&fixture)
        .arg(&prefix)
        .args([
            "--quiet",
            "--progress",
            "--max-packets",
            "1",
            "--streams",
            "0",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    let events: Vec<serde_json::Value> = String::from_utf8(result.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.last().unwrap()["done"], true);
    assert_eq!(
        events.iter().filter(|event| event["done"] == true).count(),
        1
    );
    assert_eq!(events.last().unwrap()["packets"], 1);
    let mut file = std::fs::File::open(&prefix).unwrap();
    assert_eq!(
        fvid_media::owned_wave_inspect::inspect(&mut file, None)
            .unwrap()
            .sample_frames,
        4096
    );
    let invalid = dir.join("invalid.wav");
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "loudnorm"])
        .arg(&fixture)
        .arg(&invalid)
        .args(["--loudnorm-args", "I=-16", "--loudnorm-args", "I=-20"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!invalid.exists());
    assert!(String::from_utf8_lossy(&result.stderr).contains("duplicate loudnorm option"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dual_pass_uses_owned_measurement_and_prints_one_final_report() {
    let dir = std::env::temp_dir().join(format!("fvid-dual-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/loudnorm-dual.wav");
    let output = dir.join("dual.wav");
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "loudnorm"])
        .arg(&fixture)
        .arg(&output)
        .args([
            "--dual-pass",
            "--progress",
            "--loudnorm-args",
            "I=-16:TP=-1.5:LRA=11:print_format=json",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(stats["backend"], "fvid linear loudnorm");
    assert_eq!(stats["dual_pass"], true);
    assert_eq!(stats["sample_rate"], 48000);
    assert_eq!(stats["sample_frames"], 384000);
    let messages: Vec<serde_json::Value> = String::from_utf8(result.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let reports: Vec<_> = messages
        .iter()
        .filter(|m| m.get("normalization_type").is_some())
        .collect();
    assert_eq!(reports.len(), 1);
    let report = reports[0];
    assert_eq!(report["normalization_type"], "linear");
    let number = |key: &str| report[key].as_str().unwrap().parse::<f64>().unwrap();
    assert!(number("input_lra") > 0.);
    assert!((number("output_i") + 16.).abs() < 0.1);
    assert!(number("output_tp") <= -1.49);
    let events: Vec<_> = messages
        .iter()
        .filter(|m| m.get("done").is_some())
        .collect();
    assert_eq!(events.iter().filter(|m| m["done"] == true).count(), 1);
    assert_eq!(messages.last().unwrap()["done"], true);
    for pair in events.windows(2) {
        assert!(pair[0]["packets"].as_u64().unwrap() <= pair[1]["packets"].as_u64().unwrap());
        assert!(
            pair[0]["payload_bytes"].as_u64().unwrap()
                <= pair[1]["payload_bytes"].as_u64().unwrap()
        );
    }
    assert!(output.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compressed_audio_normalization_matches_owned_pcm_with_edits_and_selection() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir =
        std::env::temp_dir().join(format!("fvid-compressed-normalizer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases = [
        ("audio/aac-mono-44k.aac", 0),
        ("audio/aac-native-edit.m4a", 0),
        ("audio/aac-stereo.mka", 0),
        ("playback-errors/alac-two-tracks.m4a", 1),
        ("playback-errors/aac-rounded-two-tracks.m4a", 1),
    ];
    for (index, (name, selected)) in cases.into_iter().enumerate() {
        let source = root.join("tests/fixtures").join(name);
        let decoded = dir.join(format!("decoded-{index}.wav"));
        let expected = dir.join(format!("expected-{index}.wav"));
        let output = dir.join(format!("output-{index}.wav"));
        fvid::native_export::export_audio_pcm_selected(
            &source,
            &decoded,
            None,
            1.,
            None,
            None,
            Some(selected),
            None,
            None,
        )
        .unwrap();
        let expected_stats = fvid_media::owned_loudnorm::apply_loudnorm_dual(
            &decoded,
            &expected,
            None,
            &Default::default(),
        )
        .unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "loudnorm"])
            .arg(&source)
            .arg(&output)
            .args([
                "--dual-pass",
                "--progress",
                "--max-rss-mib",
                "8192",
                "--max-packet-bytes",
                "65536",
                "--streams",
                &selected.to_string(),
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(stats["backend"], expected_stats.backend);
        assert_eq!(stats["sample_frames"], expected_stats.sample_frames);
        assert_eq!(stats["dual_pass"], true);
        assert_eq!(
            std::fs::read(&output).unwrap(),
            std::fs::read(&expected).unwrap(),
            "{name}"
        );
        let events: Vec<serde_json::Value> = String::from_utf8(result.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events.iter().filter(|m| m["done"] == true).count(), 1);
        assert_eq!(events.last().unwrap()["done"], true);
        for pair in events.windows(2) {
            assert!(pair[0]["packets"].as_u64().unwrap() <= pair[1]["packets"].as_u64().unwrap());
            assert!(
                pair[0]["payload_bytes"].as_u64().unwrap()
                    <= pair[1]["payload_bytes"].as_u64().unwrap()
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cancellation_after_compressed_decode_never_publishes_normalization() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-mono-44k.aac");
    let output = std::env::temp_dir().join(format!(
        "fvid-loudnorm-cancel-compressed-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&output);
    let cancel = fvid_media::CancelFlag::default();
    let hook_cancel = cancel.clone();
    let options = fvid_media::CopyOptions {
        cancel: Some(cancel),
        progress: Some(fvid_media::ProgressHook::new(move |event| {
            assert!(!event.done);
            if event.packets >= 7 {
                hook_cancel.cancel();
            }
        })),
        ..Default::default()
    };
    assert!(fvid::native_loudnorm::try_apply(&source, &output, None, true, &options).is_err());
    assert!(!output.exists());
}

#[test]
fn compressed_rss_limit_refuses_before_decode_or_publication() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-mono-44k.aac");
    let output = std::env::temp_dir().join(format!("fvid-loudnorm-rss-{}.wav", std::process::id()));
    let _ = std::fs::remove_file(&output);
    let options = fvid_media::CopyOptions {
        max_rss_bytes: Some(1),
        ..Default::default()
    };
    let error =
        fvid::native_loudnorm::try_apply(&source, &output, None, true, &options).unwrap_err();
    assert!(error.to_string().contains("rss budget exceeded"), "{error}");
    assert!(!output.exists());
}

#[test]
fn compressed_packet_limit_refuses_without_publication_in_each_container() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (index, name) in [
        "audio/aac-mono-44k.aac",
        "audio/aac-native-edit.m4a",
        "audio/aac-stereo.mka",
        "playback-errors/alac-two-tracks.m4a",
    ]
    .into_iter()
    .enumerate()
    {
        let source = root.join("tests/fixtures").join(name);
        let output = std::env::temp_dir().join(format!(
            "fvid-packet-refusal-{}-{index}.wav",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        let options = fvid_media::CopyOptions {
            max_packet_bytes: 1,
            progress: Some(fvid_media::ProgressHook::new(|event| assert!(!event.done))),
            ..Default::default()
        };
        let error =
            fvid::native_loudnorm::try_apply(&source, &output, None, true, &options).unwrap_err();
        assert!(
            error.to_string().contains("packet exceeds budget"),
            "{name}: {error}"
        );
        assert!(!output.exists());
    }
}

#[test]
fn encoded_packet_prefix_preserves_presentation_and_is_not_a_pcm_block_limit() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("fvid-encoded-prefix-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Expected frame counts include MP4 priming, rounded windows and repeated
    // ranges. Decoder preroll and repeated packet work count toward the limit.
    for (index, (name, selected, packets, frames)) in [
        ("audio/aac-mono-44k.aac", 0, 3, 3072),
        ("audio/aac-native-edit.m4a", 0, 3, 2048),
        ("audio/aac-stereo.mka", 0, 3, 2048),
        ("playback-errors/aac-rounded-two-tracks.m4a", 1, 3, 2056),
        ("playback-errors/alac-two-tracks.m4a", 1, 2, 8192),
        ("playback-errors/aac-gap-repeat-offset.m4a", 0, 6, 2296),
        ("playback-errors/aac-gap-repeat-offset.m4a", 0, 12, 7096),
    ]
    .into_iter()
    .enumerate()
    {
        let source = root.join("tests/fixtures").join(name);
        let full = dir.join(format!("full-{index}.wav"));
        let prefix = dir.join(format!("prefix-{index}.wav"));
        let expected = dir.join(format!("expected-{index}.wav"));
        let output = dir.join(format!("output-{index}.wav"));
        let decoded = fvid::native_export::export_audio_pcm_selected(
            &source,
            &full,
            None,
            1.,
            None,
            None,
            Some(selected),
            None,
            None,
        )
        .unwrap();
        let mut file = std::fs::File::open(&full).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        let bytes = std::fs::read(&full).unwrap();
        let payload_bytes = frames * usize::from(decoded.channels) * 4;
        let offset = info.data_offset as usize;
        let mut clipped = bytes[..offset + payload_bytes].to_vec();
        let riff_size = (clipped.len() - 8) as u32;
        clipped[4..8].copy_from_slice(&riff_size.to_le_bytes());
        clipped[offset - 4..offset].copy_from_slice(&(payload_bytes as u32).to_le_bytes());
        let mut at = 12;
        while at < offset - 8 {
            let size = u32::from_le_bytes(clipped[at + 4..at + 8].try_into().unwrap()) as usize;
            if &clipped[at..at + 4] == b"fact" {
                clipped[at + 8..at + 12].copy_from_slice(&(frames as u32).to_le_bytes());
            }
            at += 8 + size + (size & 1);
        }
        std::fs::write(&prefix, clipped).unwrap();
        let expected_stats = fvid_media::owned_loudnorm::apply_loudnorm_dual(
            &prefix,
            &expected,
            None,
            &Default::default(),
        )
        .unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "loudnorm"])
            .arg(&source)
            .arg(&output)
            .args([
                "--dual-pass",
                "--max-packet-bytes",
                if name.contains("alac") {
                    "65536"
                } else {
                    "1024"
                },
                "--max-packets",
                &packets.to_string(),
                "--streams",
                &selected.to_string(),
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(stats["backend"], expected_stats.backend);
        assert_eq!(
            stats["sample_frames"], expected_stats.sample_frames,
            "{name}"
        );
        let actual = std::fs::read(output).unwrap();
        let expected = std::fs::read(expected).unwrap();
        assert!(
            actual == expected,
            "{name}: first byte difference {:?}; lengths {} vs {}",
            actual.iter().zip(&expected).position(|(a, b)| a != b),
            actual.len(),
            expected.len()
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn synthetic_prefix_stops_before_truncated_tail_and_zero_count_never_publishes() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let source = root.join("aac-packet-prefix.aac");
    let video = root.join("aac-packet-prefix.y4m");
    assert_eq!(
        fvid::native_media::decode_video(&video)
            .unwrap()
            .video_frames,
        3
    );
    let output = std::env::temp_dir().join(format!("fvid-short-prefix-{}.wav", std::process::id()));
    let _ = std::fs::remove_file(&output);
    let error =
        fvid::native_loudnorm::try_apply(&source, &output, None, false, &Default::default())
            .unwrap_err();
    assert!(error.to_string().contains("fill whole buffer"), "{error}");
    assert!(!output.exists());
    let options = fvid_media::CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    assert!(fvid::native_loudnorm::try_apply(&source, &output, None, false, &options).is_err());
    assert!(!output.exists());
    let options = fvid_media::CopyOptions {
        max_packets: Some(3),
        ..Default::default()
    };
    let stats = fvid::native_loudnorm::try_apply(&source, &output, None, false, &options)
        .unwrap()
        .unwrap();
    assert_eq!(stats.backend, "fvid short loudnorm");
    assert_eq!(stats.sample_frames, (3072u64 * 192000).div_ceil(44100));
    std::fs::remove_file(output).unwrap();
}
