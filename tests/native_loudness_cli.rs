use std::process::Command;
#[test]
fn loudness_cli_uses_owned_file_meter_and_rejects_invalid_options() {
    let source = std::env::temp_dir().join(format!("fvid-loudness-cli-{}.wav", std::process::id()));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("normalized.wav"));
            let _ = std::fs::remove_file(self.0.with_extension("limited.wav"));
            let _ = std::fs::remove_file(self.0.with_extension("cli.wav"));
            let _ = std::fs::remove_file(self.0.with_extension("media.wav"));
            let _ = std::fs::remove_file(self.0.with_extension("controlled.wav"));
            let _ = std::fs::remove_file(self.0.with_extension("cancelled.wav"));
            for ext in ["normalized.mka", "normalized.f32le", "matroska.f32le", "cli.mka"] {let _ = std::fs::remove_file(self.0.with_extension(ext));}
        }
    }
    let _cleanup = Cleanup(source.clone());
    let samples: Vec<i16> = (0..96000)
        .map(|i| ((i as f64 * 0.13).sin() * 3000.0) as i16)
        .collect();
    let length = (samples.len() * 2) as u32;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((length + 36).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    for value in [1u16, 1] {
        wav.extend(value.to_le_bytes());
    }
    wav.extend(48000u32.to_le_bytes());
    wav.extend(96000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(length.to_le_bytes());
    for sample in samples {
        wav.extend(sample.to_le_bytes());
    }
    std::fs::write(&source, wav).unwrap();
    let run = |options: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "loudness"])
            .arg(&source)
            .args(options)
            .output()
            .unwrap()
    };
    let result = run(&[]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["backend"], "fvid");
    assert!(json["range_lu"].is_null());
    assert_eq!(json["sample_frames"], 96000);
    let api = fvid::native_pcm::measure_loudness_file(&source, None, &[1.0], None).unwrap();
    assert_eq!(json["integrated_lufs"].as_f64(), api.integrated_lufs);
    {
        let result = fvid::media::measure_loudness(&source, &Default::default()).unwrap();
        assert_eq!(result.integrated_lufs, api.integrated_lufs);
        assert_eq!(result.sample_peak_dbfs, api.sample_peak_dbfs);
        let result =
            fvid::media::measure_loudness_with_weights(&source, &Default::default(), &[1.0])
                .unwrap();
        assert_eq!(result.integrated_lufs, api.integrated_lufs);
    }

    assert_eq!(json["sample_peak_dbfs"].as_f64(), api.sample_peak_dbfs);
    let quiet = run(&["--quiet"]);
    assert!(quiet.status.success());
    assert!(quiet.stdout.is_empty());
    for options in [
        &["--streams", "1"][..],
        &["--channel-weights", "1,1"][..],
        &["--channel-weights", "NaN"][..],
        &["--unknown"][..],
        &["--streams", "0", "--streams", "0"][..],
    ] {
        assert!(!run(options).status.success(), "{options:?}");
    }
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "loudness"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(plan["command"], "loudness");
    assert!(plan["graph"].is_null());
    assert!(!plan["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["action"] == "write"));
    assert_eq!(plan["streams"][0]["index"], 0);
    assert_eq!(
        serde_json::to_value(fvid::media::plan_loudness(&source, &Default::default()).unwrap())
            .unwrap(),
        plan
    );
    let progress = run(&["--progress"]);
    assert!(progress.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&progress.stdout).unwrap(),
        json
    );
    let progress: Vec<serde_json::Value> = String::from_utf8(progress.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        progress
            .iter()
            .filter(|event| event["done"] == true)
            .count(),
        1
    );
    assert_eq!(progress.last().unwrap()["done"], true);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = events.clone();
    let hook =
        fvid::media_control::ProgressHook::new(move |event| captured.lock().unwrap().push(event));
    let controlled = fvid::native_pcm::measure_loudness_file_controlled(
        &source,
        None,
        &[1.0],
        None,
        Some(&hook),
    )
    .unwrap();
    assert_eq!(controlled.integrated_lufs, api.integrated_lufs);
    let captured = events.lock().unwrap();
    assert_eq!(captured.iter().filter(|event| event.done).count(), 1);
    assert!(captured.last().unwrap().done);
    assert!(captured.last().unwrap().payload_bytes > 0);
    assert!(captured
        .windows(2)
        .all(|events| events[0].packets <= events[1].packets
            && events[0].payload_bytes <= events[1].payload_bytes));
    drop(captured);
    let cancelled = fvid::media_control::CancelFlag::new();
    let signal = cancelled.clone();
    let hook = fvid::media_control::ProgressHook::new(move |event| {
        if event.payload_bytes > 0 {
            signal.cancel();
        }
    });
    assert!(fvid::native_pcm::measure_loudness_file_controlled(
        &source,
        None,
        &[1.0],
        Some(&cancelled),
        Some(&hook)
    )
    .is_err());
    {
        let options = fvid::media::CopyOptions {
            progress: Some(fvid::media_control::ProgressHook::new(|_| {})),
            ..Default::default()
        };
        assert!(fvid::media::measure_loudness(&source, &options).is_ok());
    }
    let normalized = source.with_extension("normalized.wav");
    let target = fvid::native_pcm::NormalizeTarget {
        integrated_lufs: -20.0,
        sample_peak_dbfs: 0.0,
    };
    let report =
        fvid::native_pcm::normalize_loudness_file(&source, &normalized, None, &[1.0], target, None)
            .unwrap();
    assert!(!report.peak_limited);
    assert_eq!(report.sample_frames, 96000);
    let matroska = source.with_extension("normalized.mka");
    let mka_report =
        fvid::native_pcm::normalize_loudness_file(&source, &matroska, None, &[1.0], target, None)
            .unwrap();
    assert_eq!(mka_report.gain_db, report.gain_db);
    assert_eq!(mka_report.sample_frames, report.sample_frames);
    let mut outputs = Vec::new();
    for (input, ext) in [
        (&normalized, "normalized.f32le"),
        (&matroska, "matroska.f32le"),
    ] {
        let raw = source.with_extension(ext);
        fvid::native_export::export_audio_pcm_selected(
            input, &raw, None, 1.0, None, None, None, None, None,
        )
        .unwrap();
        outputs.push(std::fs::read(raw).unwrap());
    }
    assert_eq!(outputs[0], outputs[1]);
    let cli = source.with_extension("cli.mka");
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "normalize-loudness"])
        .arg(&source)
        .arg(&cli)
        .args(["--target-lufs", "-20", "--sample-peak-dbfs", "0"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        std::fs::read(cli).unwrap(),
        std::fs::read(matroska).unwrap()
    );


    let measured =
        fvid::native_pcm::measure_loudness_file(&normalized, None, &[1.0], None).unwrap();
    assert!((measured.integrated_lufs.unwrap() + 20.0).abs() < 0.02);
    let bytes = std::fs::read(&normalized).unwrap();
    let controlled = source.with_extension("controlled.wav");
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = events.clone();
    let hook = fvid::native_pcm::NormalizationProgressHook::new(move |event| {
        captured.lock().unwrap().push(event)
    });
    fvid::native_pcm::normalize_loudness_file_controlled(
        &source,
        &controlled,
        None,
        &[1.0],
        target,
        None,
        Some(&hook),
    )
    .unwrap();
    assert_eq!(std::fs::read(&controlled).unwrap(), bytes);
    let events = events.lock().unwrap();
    assert_eq!(events.iter().filter(|event| event.done).count(), 1);
    assert_eq!(
        events.iter().filter(|event| event.phase_complete).count(),
        2
    );
    assert_eq!(
        events.first().unwrap().phase,
        fvid::native_pcm::NormalizationPhase::Measure
    );
    assert_eq!(
        events.last().unwrap().phase,
        fvid::native_pcm::NormalizationPhase::Export
    );
    assert!(events.last().unwrap().done);
    drop(events);
    let cancelled = fvid::media_control::CancelFlag::new();
    let signal = cancelled.clone();
    let hook = fvid::native_pcm::NormalizationProgressHook::new(move |event| {
        if event.phase == fvid::native_pcm::NormalizationPhase::Export && !event.done {
            signal.cancel();
        }
    });
    let output = source.with_extension("cancelled.wav");
    assert!(fvid::native_pcm::normalize_loudness_file_controlled(
        &source,
        &output,
        None,
        &[1.0],
        target,
        Some(&cancelled),
        Some(&hook)
    )
    .is_err());
    assert!(!output.exists());

    {
        let output = source.with_extension("media.wav");
        let published = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = published.clone();
        let destination = output.clone();
        let hook = fvid::native_pcm::NormalizationProgressHook::new(move |event| {
            if event.done {
                assert!(destination.exists());
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let result =
            fvid::media::normalize_loudness_controlled(&source, &output, &Default::default(), None, target, Some(&hook))
                .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        assert!(published.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(result.gain_db, report.gain_db);
        assert_eq!(result.peak_limited, report.peak_limited);
        assert_eq!(
            serde_json::to_value(&result).unwrap()["sample_frames"],
            96000
        );
        assert!(fvid::media::normalize_loudness(
            &source,
            &output,
            &Default::default(),
            None,
            target
        )
        .is_err());
    }

    assert!(fvid::native_pcm::normalize_loudness_file(
        &source,
        &normalized,
        None,
        &[1.0],
        target,
        None
    )
    .is_err());
    assert_eq!(std::fs::read(&normalized).unwrap(), bytes);
    let cli = source.with_extension("cli.wav");
    let plan_result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "normalize-loudness"])
        .arg(&source)
        .arg(&cli)
        .args(["--target-lufs", "-20", "--sample-peak-dbfs", "0"])
        .output()
        .unwrap();
    assert!(
        plan_result.status.success(),
        "{}",
        String::from_utf8_lossy(&plan_result.stderr)
    );
    assert!(!cli.exists());
    let plan: serde_json::Value = serde_json::from_slice(&plan_result.stdout).unwrap();
    assert_eq!(plan["command"], "normalize-loudness");
    let api_plan =
        fvid::native_plan::normalize_loudness(&source, None, Some(&[1.0]), target).unwrap();
    assert_eq!(plan, serde_json::to_value(api_plan).unwrap());
    assert!(fvid::native_plan::normalize_loudness(
        &source,
        None,
        None,
        fvid::native_pcm::NormalizeTarget {
            integrated_lufs: f64::NAN,
            sample_peak_dbfs: 0.0
        }
    )
    .is_err());
    assert_eq!(
        plan,
        serde_json::to_value(
            fvid::media::plan_normalize_loudness(
                &source,
                &Default::default(),
                Some(&[1.0]),
                target
            )
            .unwrap()
        )
        .unwrap()
    );
    let normalize = |options: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "normalize-loudness"])
            .arg(&source)
            .arg(&cli)
            .args(options)
            .output()
            .unwrap()
    };
    for options in [
        &["--target-lufs", "NaN"][..],
        &["--target-lufs", "1"][..],
        &["--target-lufs", "-20", "--target-lufs", "-18"][..],
        &["--unknown"][..],
    ] {
        assert!(!normalize(options).status.success());
        assert!(!cli.exists());
    }
    let result = normalize(&[
        "--target-lufs",
        "-20",
        "--sample-peak-dbfs",
        "0",
        "--progress",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read(&cli).unwrap(), bytes);
    let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(stats["backend"], "fvid");
    assert_eq!(stats["peak_limited"], false);
    let events: Vec<serde_json::Value> = String::from_utf8(result.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events.iter().filter(|event| event["done"] == true).count(),
        1
    );
    assert_eq!(events.first().unwrap()["phase"], "measure");
    assert_eq!(events.last().unwrap()["phase"], "export");
    assert!(!normalize(&[]).status.success());
    let limited = source.with_extension("limited.wav");
    let target = fvid::native_pcm::NormalizeTarget {
        integrated_lufs: 0.0,
        sample_peak_dbfs: -30.0,
    };
    let report =
        fvid::native_pcm::normalize_loudness_file(&source, &limited, None, &[1.0], target, None)
            .unwrap();
    assert!(report.peak_limited);
    let measured = fvid::native_pcm::measure_loudness_file(&limited, None, &[1.0], None).unwrap();
    assert!((measured.sample_peak_dbfs.unwrap() + 30.0).abs() < 0.001);
    assert!(measured.integrated_lufs.unwrap() < -30.0);
    let result = run(&["--streams", "0", "--channel-weights", "1"]);
    assert!(result.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap(),
        json
    );
}
