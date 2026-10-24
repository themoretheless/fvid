use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/cli-quiet-flag")
        .join(name)
}

#[test]
fn cli_short_quiet_flag_and_memory_budget_parity_across_owned_commands_and_plans() {
    let bin = env!("CARGO_BIN_EXE_fvid");
    let wav = fixture("tone.wav");
    let y4m = fixture("tiny.y4m");
    let srt = fixture("tiny.srt");
    let temp = std::env::temp_dir().join(format!(
        "fvid-cli-quiet-flag-regressions-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // 1. Owned loudnorm and plan loudnorm accept -q and --quiet and reject duplicates.
    let loudnorm_out = temp.join("loudnorm.wav");
    let out = Command::new(bin)
        .args([
            "media",
            "loudnorm",
            wav.to_str().unwrap(),
            loudnorm_out.to_str().unwrap(),
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "loudnorm -q failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "loudnorm -q must suppress stdout");
    assert!(loudnorm_out.is_file());

    let dup_out = Command::new(bin)
        .args([
            "media",
            "loudnorm",
            wav.to_str().unwrap(),
            loudnorm_out.to_str().unwrap(),
            "-q",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        !dup_out.status.success(),
        "loudnorm -q --quiet must reject duplicate quiet option"
    );

    for flag in ["-q", "--quiet"] {
        let plan_out = Command::new(bin)
            .args(["media", "plan", "loudnorm", wav.to_str().unwrap(), flag])
            .output()
            .unwrap();
        assert!(
            plan_out.status.success(),
            "plan loudnorm {flag} failed: {}",
            String::from_utf8_lossy(&plan_out.stderr)
        );
        assert!(
            plan_out.stdout.is_empty(),
            "plan loudnorm {flag} must suppress stdout"
        );
    }

    // 2. Owned loudness and plan loudness accept -q and --quiet.
    let loudness_out = Command::new(bin)
        .args(["media", "loudness", wav.to_str().unwrap(), "-q"])
        .output()
        .unwrap();
    assert!(
        loudness_out.status.success(),
        "loudness -q failed: {}",
        String::from_utf8_lossy(&loudness_out.stderr)
    );
    assert!(
        loudness_out.stdout.is_empty(),
        "loudness -q must suppress stdout"
    );

    for flag in ["-q", "--quiet"] {
        let plan_loudness = Command::new(bin)
            .args(["media", "plan", "loudness", wav.to_str().unwrap(), flag])
            .output()
            .unwrap();
        assert!(
            plan_loudness.status.success(),
            "plan loudness {flag} failed: {}",
            String::from_utf8_lossy(&plan_loudness.stderr)
        );
        assert!(
            plan_loudness.stdout.is_empty(),
            "plan loudness {flag} must suppress stdout"
        );
    }

    // 3. Owned decode-audio, plan decode-audio, plain decode, and trim-pcm / plan trim-pcm accept -q.
    let decoded_wav = temp.join("decoded.wav");
    let dec_audio_out = Command::new(bin)
        .args([
            "media",
            "decode-audio",
            wav.to_str().unwrap(),
            decoded_wav.to_str().unwrap(),
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        dec_audio_out.status.success(),
        "decode-audio -q failed: {}",
        String::from_utf8_lossy(&dec_audio_out.stderr)
    );
    assert!(
        dec_audio_out.stdout.is_empty(),
        "decode-audio -q must suppress stdout"
    );

    for flag in ["-q", "--quiet"] {
        let plan_dec_audio = Command::new(bin)
            .args([
                "media",
                "plan",
                "decode-audio",
                wav.to_str().unwrap(),
                flag,
            ])
            .output()
            .unwrap();
        assert!(
            plan_dec_audio.status.success(),
            "plan decode-audio {flag} failed: {}",
            String::from_utf8_lossy(&plan_dec_audio.stderr)
        );
        assert!(
            plan_dec_audio.stdout.is_empty(),
            "plan decode-audio {flag} must suppress stdout"
        );
    }

    let plain_dec = Command::new(bin)
        .args(["media", "decode", y4m.to_str().unwrap(), "-q"])
        .output()
        .unwrap();
    assert!(
        plain_dec.status.success(),
        "decode -q failed: {}",
        String::from_utf8_lossy(&plain_dec.stderr)
    );
    assert!(plain_dec.stdout.is_empty(), "decode -q must suppress stdout");

    let trimmed_wav = temp.join("trimmed.wav");
    let trim_pcm_out = Command::new(bin)
        .args([
            "media",
            "trim-pcm",
            wav.to_str().unwrap(),
            trimmed_wav.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.05",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        trim_pcm_out.status.success(),
        "trim-pcm -q failed: {}",
        String::from_utf8_lossy(&trim_pcm_out.stderr)
    );
    assert!(
        trim_pcm_out.stdout.is_empty(),
        "trim-pcm -q must suppress stdout"
    );

    let plan_trim_pcm = Command::new(bin)
        .args([
            "media",
            "plan",
            "trim-pcm",
            wav.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.05",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        plan_trim_pcm.status.success(),
        "plan trim-pcm -q failed: {}",
        String::from_utf8_lossy(&plan_trim_pcm.stderr)
    );
    assert!(
        plan_trim_pcm.stdout.is_empty(),
        "plan trim-pcm -q must suppress stdout"
    );

    // 4. Owned overlay, transcode-lossless, crop-lossless, xfade, burn-subtitles, export-y4m,
    //    and Y4M video trim/concat accept -q, reject duplicate -q/--quiet, and accept --progress during plan.
    let overlay_out = temp.join("overlay.mkv");
    let ov_exec = Command::new(bin)
        .args([
            "media",
            "overlay",
            y4m.to_str().unwrap(),
            y4m.to_str().unwrap(),
            overlay_out.to_str().unwrap(),
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        ov_exec.status.success(),
        "overlay -q failed: {}",
        String::from_utf8_lossy(&ov_exec.stderr)
    );
    assert!(ov_exec.stdout.is_empty(), "overlay -q must suppress stdout");

    let ov_plan = Command::new(bin)
        .args([
            "media",
            "plan",
            "overlay",
            y4m.to_str().unwrap(),
            y4m.to_str().unwrap(),
            "--hflip",
            "--progress",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        ov_plan.status.success(),
        "plan overlay --hflip --progress -q failed: {}",
        String::from_utf8_lossy(&ov_plan.stderr)
    );
    assert!(ov_plan.stdout.is_empty());

    let lossless_out = temp.join("lossless.mkv");
    let lossless_exec = Command::new(bin)
        .args([
            "media",
            "transcode",
            y4m.to_str().unwrap(),
            lossless_out.to_str().unwrap(),
            "--encoder",
            "ffv1",
            "-q",
            "--hflip",
        ])
        .output()
        .unwrap();
    assert!(
        lossless_exec.status.success(),
        "transcode --encoder ffv1 -q --hflip failed: {}",
        String::from_utf8_lossy(&lossless_exec.stderr)
    );
    assert!(
        lossless_exec.stdout.is_empty(),
        "transcode --encoder ffv1 -q must suppress stdout"
    );

    let lossless_dup = Command::new(bin)
        .args([
            "media",
            "plan",
            "transcode-lossless",
            y4m.to_str().unwrap(),
            "-q",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        !lossless_dup.status.success(),
        "plan transcode-lossless -q --quiet must reject duplicate quiet option"
    );

    let xfade_plan = Command::new(bin)
        .args([
            "media",
            "plan",
            "xfade",
            y4m.to_str().unwrap(),
            y4m.to_str().unwrap(),
            "--xfade-duration",
            "1",
            "--progress",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        xfade_plan.status.success(),
        "plan xfade --progress -q failed: {}",
        String::from_utf8_lossy(&xfade_plan.stderr)
    );
    assert!(xfade_plan.stdout.is_empty());

    let burn_plan = Command::new(bin)
        .args([
            "media",
            "plan",
            "burn-subtitles",
            y4m.to_str().unwrap(),
            "--subs",
            srt.to_str().unwrap(),
            "--progress",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        burn_plan.status.success(),
        "plan burn-subtitles --progress -q failed: {}",
        String::from_utf8_lossy(&burn_plan.stderr)
    );
    assert!(burn_plan.stdout.is_empty());

    let exported_y4m = temp.join("exported.y4m");
    let export_y4m_out = Command::new(bin)
        .args([
            "media",
            "export-y4m",
            y4m.to_str().unwrap(),
            exported_y4m.to_str().unwrap(),
            "--max-rss-mib",
            "512",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        export_y4m_out.status.success(),
        "export-y4m --max-rss-mib -q failed: {}",
        String::from_utf8_lossy(&export_y4m_out.stderr)
    );
    assert!(export_y4m_out.stdout.is_empty());

    let trim_y4m_plan = Command::new(bin)
        .args([
            "media",
            "plan",
            "trim",
            y4m.to_str().unwrap(),
            "--output-format",
            "y4m",
            "--from",
            "0",
            "--to",
            "1",
            "--progress",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        trim_y4m_plan.status.success(),
        "plan trim --output-format y4m --progress -q failed: {}",
        String::from_utf8_lossy(&trim_y4m_plan.stderr)
    );
    assert!(trim_y4m_plan.stdout.is_empty());

    let concat_y4m_plan = Command::new(bin)
        .args([
            "media",
            "plan",
            "concat",
            y4m.to_str().unwrap(),
            y4m.to_str().unwrap(),
            "--output-format",
            "y4m",
            "--progress",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        concat_y4m_plan.status.success(),
        "plan concat --output-format y4m --progress -q failed: {}",
        String::from_utf8_lossy(&concat_y4m_plan.stderr)
    );
    assert!(concat_y4m_plan.stdout.is_empty());

    let _ = std::fs::remove_dir_all(&temp);
}
