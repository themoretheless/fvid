use std::path::{Path, PathBuf};
use std::process::Command;

use fvid::media::{self, AudioDecodeTransform, CopyOptions};

struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-decode-trim-wave-budget-parity-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/decode-trim-wave-budget-parity")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn decode_audio_and_trim_enforce_packet_bytes_and_zero_packet_limits_across_containers() {
    let dir = fixture_dir();
    let wav = dir.join("stereo-48k.wav");
    let adts = dir.join("adts-mono-48k.aac");
    let mp4 = dir.join("mp4-pcm-48k.m4a");
    let mka = dir.join("matroska-pcm-48k.mka");
    let tmp = Directory::new("decode-trim-budget");

    let tiny_pkt = CopyOptions {
        max_packet_bytes: 2,
        ..CopyOptions::default()
    };
    let zero_pkts = CopyOptions {
        max_packets: Some(0),
        ..CopyOptions::default()
    };
    let zero_pkts_with_controlled = CopyOptions {
        max_packets: Some(0),
        max_controlled_bytes: Some(1_048_576),
        ..CopyOptions::default()
    };

    for (name, path, out_ext) in [
        ("wav", &wav, "wav"),
        ("adts", &adts, "aac"),
        ("mp4", &mp4, "m4a"),
        ("mka", &mka, "mka"),
    ] {
        for (label, opts) in [
            ("tiny_pkt", &tiny_pkt),
            ("zero_pkts", &zero_pkts),
            ("zero_pkts_with_controlled", &zero_pkts_with_controlled),
        ] {
            // 1. media::plan_decode_audio and fvid_media::plan_decode_audio must reject
            assert!(
                media::plan_decode_audio(path, &AudioDecodeTransform::default(), opts).is_err(),
                "media::plan_decode_audio must reject {label} on {name}"
            );
            assert!(
                fvid_media::plan_decode_audio(path, &AudioDecodeTransform::default(), opts)
                    .is_err(),
                "fvid_media::plan_decode_audio must reject {label} on {name}"
            );

            // 2. media::decode_audio_transformed must reject without creating output files
            for decode_ext in ["wav", "f32le", "mka"] {
                let out_decode = tmp.0.join(format!("decode-{name}-{label}.{decode_ext}"));
                assert!(
                    media::decode_audio_transformed(
                        path,
                        &out_decode,
                        AudioDecodeTransform::default(),
                        opts,
                    )
                    .is_err(),
                    "media::decode_audio_transformed (.{decode_ext}) must reject {label} on {name}"
                );
                assert!(
                    !out_decode.exists(),
                    "decode output should not exist after failure: {}",
                    out_decode.display()
                );
            }

            // 3. media::plan_trim and media::trim must reject without creating output files
            assert!(
                media::plan_trim(path, 0, 20_000, opts).is_err(),
                "media::plan_trim must reject {label} on {name}"
            );
            let out_trim = tmp.0.join(format!("trim-{name}-{label}.{out_ext}"));
            assert!(
                media::trim(path, &out_trim, 0, 20_000, opts).is_err(),
                "media::trim must reject {label} on {name}"
            );
            assert!(
                !out_trim.exists(),
                "trim output should not exist after failure: {}",
                out_trim.display()
            );
        }

        // 4. CLI parity for `plan decode-audio`, `decode-audio`, `plan trim`, and `trim`
        for (flag, val) in [("--max-packet-bytes", "2"), ("--max-packets", "0")] {
            let cli_plan_decode = Command::new(bin_path())
                .args([
                    "media",
                    "plan",
                    "decode-audio",
                    path.to_str().unwrap(),
                    flag,
                    val,
                ])
                .output()
                .expect("cli plan decode-audio");
            assert!(
                !cli_plan_decode.status.success(),
                "CLI plan decode-audio must reject {flag} {val} on {name}"
            );

            let out_cli_decode = tmp.0.join(format!("cli-decode-{name}-{flag}-{val}.wav"));
            let cli_decode = Command::new(bin_path())
                .args([
                    "media",
                    "decode-audio",
                    path.to_str().unwrap(),
                    out_cli_decode.to_str().unwrap(),
                    flag,
                    val,
                ])
                .output()
                .expect("cli decode-audio");
            assert!(
                !cli_decode.status.success(),
                "CLI decode-audio must reject {flag} {val} on {name}"
            );
            assert!(!out_cli_decode.exists());

            let cli_plan_trim = Command::new(bin_path())
                .args([
                    "media",
                    "plan",
                    "trim",
                    path.to_str().unwrap(),
                    "--from",
                    "0",
                    "--to",
                    "0.02",
                    flag,
                    val,
                ])
                .output()
                .expect("cli plan trim");
            assert!(
                !cli_plan_trim.status.success(),
                "CLI plan trim must reject {flag} {val} on {name}"
            );

            let out_cli_trim = tmp
                .0
                .join(format!("cli-trim-{name}-{flag}-{val}.{out_ext}"));
            let cli_trim = Command::new(bin_path())
                .args([
                    "media",
                    "trim",
                    path.to_str().unwrap(),
                    out_cli_trim.to_str().unwrap(),
                    "--from",
                    "0",
                    "--to",
                    "0.02",
                    flag,
                    val,
                ])
                .output()
                .expect("cli trim");
            assert!(
                !cli_trim.status.success(),
                "CLI trim must reject {flag} {val} on {name}"
            );
            assert!(!out_cli_trim.exists());
        }
    }
}

#[test]
fn wave_trim_trim_pcm_and_concat_preserve_stream_zero_errors_backend_and_budget_notes() {
    let dir = fixture_dir();
    let wav1 = dir.join("stereo-48k.wav");
    let wav2 = dir.join("second-stereo-48k.wav");
    let tmp = Directory::new("wave-parity");

    let stream1_default = CopyOptions {
        streams: vec![1],
        ..CopyOptions::default()
    };
    let stream1_with_budget = CopyOptions {
        streams: vec![1],
        max_packets: Some(10),
        max_controlled_bytes: Some(1_048_576),
        ..CopyOptions::default()
    };

    for (label, opts) in [
        ("default", &stream1_default),
        ("with_budget", &stream1_with_budget),
    ] {
        let trim_plan_err = media::plan_trim(&wav1, 0, 20_000, opts)
            .expect_err("plan_trim on WAVE stream 1 must fail");
        assert!(
            trim_plan_err.to_string().contains("WAVE has only stream 0"),
            "unexpected plan_trim error ({label}): {trim_plan_err}"
        );

        let trim_pcm_plan_err = media::plan_trim_pcm(&wav1, 0, 20_000, opts)
            .expect_err("plan_trim_pcm on WAVE stream 1 must fail");
        assert!(
            trim_pcm_plan_err
                .to_string()
                .contains("WAVE has only stream 0"),
            "unexpected plan_trim_pcm error ({label}): {trim_pcm_plan_err}"
        );

        let out_trim_pcm = tmp.0.join(format!("trim-pcm-{label}.wav"));
        let trim_pcm_err = media::trim_pcm(&wav1, &out_trim_pcm, 0, 20_000, opts)
            .expect_err("trim_pcm on WAVE stream 1 must fail");
        assert!(
            trim_pcm_err.to_string().contains("WAVE has only stream 0"),
            "unexpected trim_pcm error ({label}): {trim_pcm_err}"
        );
        assert!(!out_trim_pcm.exists());

        let concat_plan_err = media::plan_concat(&[wav1.clone(), wav2.clone()], opts)
            .expect_err("plan_concat on WAVE stream 1 must fail");
        assert!(
            concat_plan_err
                .to_string()
                .contains("WAVE has only stream 0"),
            "unexpected plan_concat error ({label}): {concat_plan_err}"
        );

        let out_concat = tmp.0.join(format!("concat-{label}.wav"));
        let concat_err = media::concat(&[wav1.clone(), wav2.clone()], &out_concat, opts)
            .expect_err("concat on WAVE stream 1 must fail");
        assert!(
            concat_err.to_string().contains("WAVE has only stream 0"),
            "unexpected concat error ({label}): {concat_err}"
        );
        assert!(!out_concat.exists());
    }

    // CLI stream 1 on WAVE trim / trim-pcm must also report "WAVE has only stream 0"
    for subcmd in ["trim", "trim-pcm"] {
        let out_cli = tmp.0.join(format!("cli-{subcmd}-stream1.wav"));
        let cli_out = Command::new(bin_path())
            .args([
                "media",
                subcmd,
                wav1.to_str().unwrap(),
                out_cli.to_str().unwrap(),
                "--from",
                "0",
                "--to",
                "0.02",
                "--streams",
                "1",
            ])
            .output()
            .expect("cli wave stream 1");
        assert!(!cli_out.status.success());
        let stderr = String::from_utf8_lossy(&cli_out.stderr);
        assert!(
            stderr.contains("WAVE has only stream 0"),
            "expected 'WAVE has only stream 0' for CLI {subcmd}, got: {stderr}"
        );
        assert!(!out_cli.exists());
    }

    // WAVE concat with custom budgets and metadata must normalize backend to "fvid"
    // and include budget notes in plan_concat, plan_trim, plan_trim_pcm, and plan_decode_audio.
    let budgeted_opts = CopyOptions {
        max_packet_bytes: 4096,
        max_packets: Some(16),
        max_controlled_bytes: Some(524_288),
        metadata_set: vec![("artist".to_string(), "FVID-0249".to_string())],
        ..CopyOptions::default()
    };

    let out_concat_ok = tmp.0.join("concat-budgeted.wav");
    let concat_stats = media::concat(&[wav1.clone(), wav2.clone()], &out_concat_ok, &budgeted_opts)
        .expect("WAVE concat with custom budget and metadata should succeed");
    assert_eq!(concat_stats.backend, "fvid");

    for (plan_name, plan) in [
        (
            "plan_decode_audio",
            media::plan_decode_audio(&wav1, &AudioDecodeTransform::default(), &budgeted_opts)
                .expect("plan_decode_audio"),
        ),
        (
            "plan_trim",
            media::plan_trim(&wav1, 0, 20_000, &budgeted_opts).expect("plan_trim"),
        ),
        (
            "plan_trim_pcm",
            media::plan_trim_pcm(&wav1, 0, 20_000, &budgeted_opts).expect("plan_trim_pcm"),
        ),
        (
            "plan_concat",
            media::plan_concat(&[wav1.clone(), wav2.clone()], &budgeted_opts).expect("plan_concat"),
        ),
    ] {
        assert!(
            plan.notes
                .iter()
                .any(|n| n.contains("encoded packet limit: 4096 bytes")
                    && n.contains("packet work limit: 16 packets")
                    && n.contains("controlled memory limit: 524288 bytes")),
            "{plan_name} missing budget note in {:?}",
            plan.notes
        );
    }
}
