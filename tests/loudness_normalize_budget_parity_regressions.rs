use fvid::media::{self, CopyOptions};
use fvid::native_pcm::NormalizeTarget;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/loudness-normalize-budget-parity")
}

fn fvid_bin() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn measure_loudness_and_normalize_loudness_enforce_zero_packet_and_packet_byte_budgets() {
    let dir = fixture_dir();
    let wav = dir.join("stereo-48k.wav");
    let adts = dir.join("adts-mono-48k.aac");
    let mp4 = dir.join("mp4-pcm-48k.m4a");
    let mka = dir.join("matroska-pcm-48k.mka");
    let mka_flac = dir.join("matroska-flac-48k.mka");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch243-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. max_packets == Some(0) is rejected with "loudness requires at least one packet" across all containers
    let mut zero_pkt = CopyOptions::default();
    zero_pkt.max_packets = Some(0);
    for (name, path) in [
        ("wav", &wav),
        ("adts", &adts),
        ("mp4", &mp4),
        ("mka", &mka),
        ("mka_flac", &mka_flac),
    ] {
        let err = media::measure_loudness(path, &zero_pkt).unwrap_err();
        assert!(
            err.contains("loudness requires at least one packet"),
            "measure_loudness zero_pkt for {name} returned: {err}"
        );
        let plan_err = media::plan_loudness(path, &zero_pkt).unwrap_err();
        assert!(
            plan_err.contains("loudness requires at least one packet"),
            "plan_loudness zero_pkt for {name} returned: {plan_err}"
        );
        let norm_plan_err =
            media::plan_normalize_loudness(path, &zero_pkt, None, NormalizeTarget::default())
                .unwrap_err();
        assert!(
            norm_plan_err.contains("loudness requires at least one packet"),
            "plan_normalize_loudness zero_pkt for {name} returned: {norm_plan_err}"
        );
        let out_wav = temp_dir.join(format!("zero-{name}.wav"));
        let norm_err = media::normalize_loudness(
            path,
            &out_wav,
            &zero_pkt,
            None,
            NormalizeTarget::default(),
        )
        .unwrap_err();
        assert!(
            norm_err.contains("loudness requires at least one packet"),
            "normalize_loudness zero_pkt for {name} returned: {norm_err}"
        );
        assert!(!out_wav.exists());
    }

    // 2. max_packet_bytes: 2 is enforced during measure_loudness and normalize_loudness
    let mut tiny_pkt = CopyOptions::default();
    tiny_pkt.max_packet_bytes = 2;
    for (name, path, expected_substr) in [
        ("wav", &wav, "packet byte limit is smaller than one PCM sample frame"),
        ("adts", &adts, "exceeds"),
        ("mp4", &mp4, "MP4 packet exceeds budget"),
        ("mka", &mka, "exceeds budget"),
        ("mka_flac", &mka_flac, "exceeds budget"),
    ] {
        let err = media::measure_loudness(path, &tiny_pkt).unwrap_err();
        assert!(
            err.contains(expected_substr),
            "measure_loudness tiny_pkt for {name} returned: {err}"
        );
        let plan_err = media::plan_loudness(path, &tiny_pkt).unwrap_err();
        assert!(
            plan_err.contains(expected_substr),
            "plan_loudness tiny_pkt for {name} returned: {plan_err}"
        );
        let norm_plan_err =
            media::plan_normalize_loudness(path, &tiny_pkt, None, NormalizeTarget::default())
                .unwrap_err();
        assert!(
            norm_plan_err.contains(expected_substr),
            "plan_normalize_loudness tiny_pkt for {name} returned: {norm_plan_err}"
        );
        let out_wav = temp_dir.join(format!("tiny-{name}.wav"));
        let norm_err = media::normalize_loudness(
            path,
            &out_wav,
            &tiny_pkt,
            None,
            NormalizeTarget::default(),
        )
        .unwrap_err();
        assert!(
            norm_err.contains(expected_substr),
            "normalize_loudness tiny_pkt for {name} returned: {norm_err}"
        );
        assert!(!out_wav.exists());
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn plan_loudness_and_loudnorm_do_not_duplicate_budget_notes_and_enforce_budgets() {
    let dir = fixture_dir();
    let wav = dir.join("stereo-48k.wav");
    let adts = dir.join("adts-mono-48k.aac");
    let mp4 = dir.join("mp4-pcm-48k.m4a");
    let mka = dir.join("matroska-pcm-48k.mka");
    let mka_flac = dir.join("matroska-flac-48k.mka");

    // 1. Single budget note on plan_loudness and plan_normalize_loudness when max_controlled_bytes is set or fvid_media::plan_decode_audio is used
    let mut mem_opts = CopyOptions::default();
    mem_opts.max_controlled_bytes = Some(64 * 1024 * 1024);
    mem_opts.max_packets = Some(10);
    for (name, path) in [
        ("wav", &wav),
        ("adts", &adts),
        ("mp4", &mp4),
        ("mka", &mka),
        ("mka_flac", &mka_flac),
    ] {
        let plan = media::plan_loudness(path, &mem_opts)
            .unwrap_or_else(|e| panic!("plan_loudness failed for {name}: {e}"));
        let budget_notes: Vec<_> = plan
            .notes
            .iter()
            .filter(|n| n.starts_with("encoded packet limit:"))
            .collect();
        assert_eq!(
            budget_notes.len(),
            1,
            "expected exactly 1 budget note on plan_loudness for {name}, got: {:?}",
            plan.notes
        );

        let norm_plan =
            media::plan_normalize_loudness(path, &mem_opts, None, NormalizeTarget::default())
                .unwrap_or_else(|e| panic!("plan_normalize_loudness failed for {name}: {e}"));
        let norm_budget_notes: Vec<_> = norm_plan
            .notes
            .iter()
            .filter(|n| n.starts_with("encoded packet limit:"))
            .collect();
        assert_eq!(
            norm_budget_notes.len(),
            1,
            "expected exactly 1 budget note on plan_normalize_loudness for {name}, got: {:?}",
            norm_plan.notes
        );
    }
}

#[test]
fn cli_loudness_and_normalize_loudness_enforce_metadata_and_budget_parity() {
    let dir = fixture_dir();
    let wav = dir.join("stereo-48k.wav");
    let mka_flac = dir.join("matroska-flac-48k.mka");

    // 1. CLI `loudness` and `plan loudness` reject metadata flags with the owned loudness contract error
    for cmd_prefix in [&["loudness"][..], &["plan", "loudness"][..]] {
        for meta_args in [
            &["--metadata", "title=test"][..],
            &["--metadata-delete", "title"][..],
            &["--stream-metadata", "0:title=test"][..],
            &["--stream-metadata-delete", "0:title"][..],
        ] {
            let mut args = vec!["media"];
            args.extend_from_slice(cmd_prefix);
            args.push(wav.to_str().unwrap());
            args.extend_from_slice(meta_args);
            let out = Command::new(fvid_bin()).args(&args).output().unwrap();
            assert!(
                !out.status.success(),
                "CLI {:?} unexpectedly succeeded with {:?}",
                cmd_prefix,
                meta_args
            );
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                stderr.contains("requires at most one audio stream and no metadata edits"),
                "unexpected stderr for {:?} {:?}: {stderr}",
                cmd_prefix,
                meta_args
            );
        }
    }

    // 2. CLI `loudness` and `plan normalize-loudness` enforce `--max-packet-bytes 2` and `--max-packets 0` on Matroska FLAC before inspecting default channel weights
    for extra in [
        &["--max-packet-bytes", "2"][..],
        &["--max-packets", "0"][..],
    ] {
        let mut args = vec!["media", "loudness", mka_flac.to_str().unwrap()];
        args.extend_from_slice(extra);
        let out = Command::new(fvid_bin()).args(&args).output().unwrap();
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        if extra[0] == "--max-packet-bytes" {
            assert!(
                stderr.contains("exceeds budget"),
                "unexpected stderr for loudness {:?}: {stderr}",
                extra
            );
        } else {
            assert!(
                stderr.contains("loudness requires at least one packet"),
                "unexpected stderr for loudness {:?}: {stderr}",
                extra
            );
        }

        let mut plan_args = vec![
            "media",
            "plan",
            "normalize-loudness",
            mka_flac.to_str().unwrap(),
        ];
        plan_args.extend_from_slice(extra);
        let plan_out = Command::new(fvid_bin()).args(&plan_args).output().unwrap();
        assert!(!plan_out.status.success());
        let plan_stderr = String::from_utf8_lossy(&plan_out.stderr);
        if extra[0] == "--max-packet-bytes" {
            assert!(
                plan_stderr.contains("exceeds budget"),
                "unexpected stderr for plan normalize-loudness {:?}: {plan_stderr}",
                extra
            );
        } else {
            assert!(
                plan_stderr.contains("loudness requires at least one packet"),
                "unexpected stderr for plan normalize-loudness {:?}: {plan_stderr}",
                extra
            );
        }
    }
}
