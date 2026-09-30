use std::process::Command;
#[test]
fn loudness_cli_uses_owned_file_meter_and_rejects_invalid_options() {
    let source = std::env::temp_dir().join(format!("fvid-loudness-cli-{}.wav", std::process::id()));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
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
    assert_eq!(json["sample_frames"], 96000);
    let api = fvid::native_pcm::measure_loudness_file(&source, None, &[1.0], None).unwrap();
    assert_eq!(json["integrated_lufs"].as_f64(), api.integrated_lufs);
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
    let result = run(&["--streams", "0", "--channel-weights", "1"]);
    assert!(result.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap(),
        json
    );
}
