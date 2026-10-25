use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use fvid::media::{self, AudioDecodeTransform, CopyOptions, MixAudioOptions};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("mix-merge-prefix-budget-wave-info")
        .join(name)
}

fn temp_wav_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "fvid-batch261-{}-{}-{}.wav",
        tag,
        std::process::id(),
        nanos
    ))
}

#[test]
fn mix_and_merge_audio_respect_prefix_max_packets_in_packet_byte_admission() {
    let a = fixture("prefix-a.flac");
    let b = fixture("prefix-b.flac");
    let sources = vec![a.clone(), b.clone()];
    let mix_opts = MixAudioOptions::default();

    // Packet 0 in both files is 11 bytes; packet 1 is 393 bytes.
    // With max_packets = Some(1) and max_packet_bytes = 50, only packet 0 is decoded per input,
    // so both planning and execution must succeed and emit 192 sample frames.
    let mut prefix_controls = CopyOptions::default();
    prefix_controls.max_packet_bytes = 50;
    prefix_controls.max_packets = Some(1);

    let mix_plan =
        media::plan_mix_audio_with_controls(&sources, &mix_opts, &prefix_controls)
            .expect("prefix-bounded mix-audio plan must admit 11-byte first packet");
    assert_eq!(mix_plan.command, "mix-audio");

    let mix_out = temp_wav_path("mix-prefix");
    let mix_stats =
        media::mix_audio_with_controls(&sources, &mix_out, &mix_opts, &prefix_controls)
            .expect("prefix-bounded mix-audio must decode only packet 0 and succeed");
    let _ = std::fs::remove_file(&mix_out);
    assert_eq!(mix_stats.sample_frames, 192);
    assert_eq!(mix_stats.sample_rate, 48_000);
    assert_eq!(mix_stats.channels, 1);

    let merge_plan =
        media::plan_merge_audio_with_controls(&sources, &prefix_controls)
            .expect("prefix-bounded merge-audio plan must admit 11-byte first packet");
    assert_eq!(merge_plan.command, "merge-audio");

    let merge_out = temp_wav_path("merge-prefix");
    let merge_stats =
        media::merge_audio_with_controls(&sources, &merge_out, &prefix_controls)
            .expect("prefix-bounded merge-audio must decode only packet 0 and succeed");
    let _ = std::fs::remove_file(&merge_out);
    assert_eq!(merge_stats.sample_frames, 192);
    assert_eq!(merge_stats.sample_rate, 48_000);
    assert_eq!(merge_stats.channels, 2);

    // When max_packets allows packet 1 (Some(2)), the 393-byte second packet must still be rejected.
    let mut two_packet_controls = prefix_controls.clone();
    two_packet_controls.max_packets = Some(2);
    let err = media::mix_audio_with_controls(&sources, &mix_out, &mix_opts, &two_packet_controls)
        .expect_err("second packet exceeds 50-byte packet limit");
    assert!(
        err.to_string().contains("FLAC packet bytes exceed limit"),
        "unexpected error: {err}"
    );
}

#[test]
fn wave_export_and_loudnorm_preserve_fourcc_riff_info_tags_across_flac_opus_and_mp3() {
    let opts = CopyOptions::default();

    for (name, tag, expected_abcd, expected_isrc) in [
        ("prefix-a.flac", "flac", "flac-custom-4cc", "US-FVD-26-00268"),
        ("fourcc-tags.opus", "opus", "opus-custom-4cc", "US-OPU-26-00268"),
        ("fourcc-tags.mp3", "mp3", "mp3-custom-4cc", "US-MP3-26-00268"),
    ] {
        let src = fixture(name);
        let export_out = temp_wav_path(&format!("{tag}-export"));
        fvid_media::decode_audio_transformed(
            &src,
            &export_out,
            AudioDecodeTransform::default(),
            &opts,
        )
        .unwrap_or_else(|e| panic!("{name} wave export failed: {e}"));
        let export_info = media::probe(&export_out).expect("probe exported wave");
        let _ = std::fs::remove_file(&export_out);
        assert_eq!(
            export_info.metadata.get("ABCD").map(String::as_str),
            Some(expected_abcd),
            "{name} export dropped ABCD fourcc tag: {:?}",
            export_info.metadata
        );
        assert_eq!(
            export_info.metadata.get("ISRC").map(String::as_str),
            Some(expected_isrc),
            "{name} export dropped ISRC fourcc tag: {:?}",
            export_info.metadata
        );

        let loudnorm_out = temp_wav_path(&format!("{tag}-loudnorm"));
        media::apply_loudnorm(&src, &loudnorm_out, None, &opts)
            .unwrap_or_else(|e| panic!("{name} loudnorm failed: {e}"));
        let loudnorm_info = media::probe(&loudnorm_out).expect("probe loudnorm wave");
        let _ = std::fs::remove_file(&loudnorm_out);
        assert_eq!(
            loudnorm_info.metadata.get("ABCD").map(String::as_str),
            Some(expected_abcd),
            "{name} loudnorm dropped ABCD fourcc tag: {:?}",
            loudnorm_info.metadata
        );
        assert_eq!(
            loudnorm_info.metadata.get("ISRC").map(String::as_str),
            Some(expected_isrc),
            "{name} loudnorm dropped ISRC fourcc tag: {:?}",
            loudnorm_info.metadata
        );
    }
}

#[test]
fn mix_merge_and_convert_subtitles_cli_preserve_budget_and_validation_errors_in_default_builds() {
    let a = fixture("prefix-a.flac");
    let b = fixture("prefix-b.flac");
    let a_str = a.to_str().unwrap();
    let b_str = b.to_str().unwrap();

    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-batch295-mix-merge-sub-cli-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. mix-audio and merge-audio to .mka/.mkv under tight --max-packet-bytes 1
    // must propagate the exact FLAC packet-byte budget error instead of falling through
    // to "this media operation requires cargo build --release --features media".
    let mix_mka = temp_dir.join("mix-tight.mka");
    let mix_mka_err = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "mix-audio",
            mix_mka.to_str().unwrap(),
            a_str,
            b_str,
            "--max-packet-bytes",
            "1",
        ])
        .output()
        .unwrap();
    assert!(!mix_mka_err.status.success());
    let mix_mka_stderr = String::from_utf8_lossy(&mix_mka_err.stderr);
    assert!(
        mix_mka_stderr.contains("FLAC packet bytes exceed limit"),
        "expected FLAC packet byte limit error on mix-audio .mka, got: {mix_mka_stderr}"
    );
    assert!(!mix_mka.exists());

    let merge_mka = temp_dir.join("merge-tight.mka");
    let merge_mka_err = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "merge-audio",
            merge_mka.to_str().unwrap(),
            a_str,
            b_str,
            "--max-packet-bytes",
            "1",
        ])
        .output()
        .unwrap();
    assert!(!merge_mka_err.status.success());
    let merge_mka_stderr = String::from_utf8_lossy(&merge_mka_err.stderr);
    assert!(
        merge_mka_stderr.contains("FLAC packet bytes exceed limit"),
        "expected FLAC packet byte limit error on merge-audio .mka, got: {merge_mka_stderr}"
    );
    assert!(!merge_mka.exists());

    // 2. mix-audio and merge-audio with unsupported output extension (.mp3) or unknown flags
    let bad_mix_ext = temp_dir.join("mix.mp3");
    let bad_mix_out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "mix-audio",
            bad_mix_ext.to_str().unwrap(),
            a_str,
            b_str,
        ])
        .output()
        .unwrap();
    assert!(!bad_mix_out.status.success());
    let bad_mix_stderr = String::from_utf8_lossy(&bad_mix_out.stderr);
    assert!(
        bad_mix_stderr.contains("mix-audio requires .wav, .mka or .mkv output"),
        "expected mix-audio output extension error, got: {bad_mix_stderr}"
    );

    let bad_merge_ext = temp_dir.join("merge.mp3");
    let bad_merge_out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "merge-audio",
            bad_merge_ext.to_str().unwrap(),
            a_str,
            b_str,
        ])
        .output()
        .unwrap();
    assert!(!bad_merge_out.status.success());
    let bad_merge_stderr = String::from_utf8_lossy(&bad_merge_out.stderr);
    assert!(
        bad_merge_stderr.contains("merge-audio requires .wav, .mka or .mkv output"),
        "expected merge-audio output extension error, got: {bad_merge_stderr}"
    );

    for (cmd, flag) in [
        (&["media", "mix-audio", "--bogus"][..], "unsupported mix-audio option: --bogus"),
        (&["media", "plan", "mix-audio", "--bogus"][..], "unsupported mix-audio option: --bogus"),
        (&["media", "merge-audio", "--bogus"][..], "unsupported merge-audio option: --bogus"),
        (&["media", "plan", "merge-audio", "--bogus"][..], "unsupported merge-audio option: --bogus"),
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(cmd)
            .output()
            .unwrap();
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(flag),
            "expected `{flag}` for {cmd:?}, got: {stderr}"
        );
    }

    // 3. convert-subtitles and plan convert-subtitles CLI validation on missing/extra paths,
    // unknown flags, and non-.mkv destination paths in default builds.
    let srt = temp_dir.join("sub.srt");
    std::fs::write(&srt, b"1\n00:00:00,000 --> 00:00:01,000\nHello\n").unwrap();
    let srt_str = srt.to_str().unwrap();
    let bad_sub_out = temp_dir.join("sub.mp4");

    for (cmd, expected) in [
        (
            vec!["media", "convert-subtitles"],
            "convert-subtitles requires INPUT OUTPUT.mkv",
        ),
        (
            vec!["media", "convert-subtitles", srt_str],
            "convert-subtitles requires INPUT OUTPUT.mkv",
        ),
        (
            vec!["media", "convert-subtitles", "--bogus", srt_str, "out.mkv"],
            "unsupported convert-subtitles option: --bogus",
        ),
        (
            vec!["media", "plan", "convert-subtitles"],
            "plan convert-subtitles requires INPUT [OUTPUT.mkv]",
        ),
        (
            vec![
                "media",
                "plan",
                "convert-subtitles",
                srt_str,
                bad_sub_out.to_str().unwrap(),
            ],
            "convert-subtitles requires Matroska (.mkv) output",
        ),
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&cmd)
            .output()
            .unwrap();
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(expected),
            "expected `{expected}` for {cmd:?}, got: {stderr}"
        );
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

