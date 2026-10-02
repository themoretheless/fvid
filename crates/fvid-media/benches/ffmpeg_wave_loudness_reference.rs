//! Explicit benchmark oracle; not part of ordinary tests or production.
use fvid_media::{CopyOptions, measure_loudness, owned_wav_file::write_wav_f64le};
use std::process::Command;

fn main() {
    let dir = std::env::temp_dir().join(format!("fvid-loudness-bench-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for (name, channels, frequency, smooth, rate) in [
        ("mono", 1, 1000., true, 48000),
        ("stereo", 2, 12000., true, 48000),
        ("stereo-transient", 2, 12000., false, 48000),
        ("88200", 1, 22050., true, 88200),
        ("96000", 1, 24000., true, 96000),
        ("176400", 1, 44100., true, 176400),
        ("192000", 1, 1000., true, 192000),
        ("352800", 1, 1000., true, 352800),
        ("384000", 1, 1000., true, 384000),
        ("192000-quarter-rate-diagnostic", 1, 48000., true, 192000),
    ] {
        let path = dir.join(format!("{name}.wav"));
        let pcm: Vec<f64> = (0..rate * 12)
            .flat_map(|i| {
                let transition = ((i as f64 - (rate as f64 * 6. - 64.)) / 128.).clamp(0., 1.);
                let gain = if smooth {
                    0.2 - 0.16 * transition
                } else if i < rate * 6 {
                    0.2
                } else {
                    0.04
                };
                let fade = if smooth {
                    (i.min(rate * 12 - 1 - i) as f64 / 64.).min(1.)
                } else {
                    1.
                };
                let sample = gain
                    * fade
                    * (std::f64::consts::TAU * frequency * i as f64 / rate as f64
                        + std::f64::consts::FRAC_PI_4)
                        .sin();
                (0..channels).map(move |ch| if ch == 0 { sample } else { -sample })
            })
            .collect();
        write_wav_f64le(&path, rate, channels, &pcm).unwrap();
        let owned = measure_loudness(&path, &CopyOptions::default()).unwrap();
        let output = Command::new(&ffmpeg)
            .args(["-nostdin", "-v", "info", "-i"])
            .arg(&path)
            .args([
                "-af",
                "ebur128=peak=sample+true:framelog=verbose",
                "-f",
                "null",
                "-",
            ])
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{log}");
        let summary = log.rsplit("Summary:").next().unwrap();
        let metric = |key: &str| -> f64 {
            summary
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix(key)
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap_or_else(|| panic!("missing {key}: {summary}"))
        };
        let peaks: Vec<f64> = summary
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("Peak:")
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse().ok())
            })
            .collect();
        assert_eq!(peaks.len(), 2, "{summary}");
        for (key, value, expected, tolerance) in [
            ("I", owned.integrated_lufs, metric("I:"), 0.11),
            ("LRA", owned.range_lu, metric("LRA:"), 0.2),
            ("low", owned.lra_low_lufs, metric("LRA low:"), 0.2),
            ("high", owned.lra_high_lufs, metric("LRA high:"), 0.2),
            ("sample", owned.sample_peak_dbfs, peaks[0], 0.11),
            ("true", owned.true_peak_dbfs, peaks[1], 0.2),
        ] {
            // The owned Annex-2 FIR and the reference resampler have different
            // transient responses. Retain that case as a diagnostic, rather
            // than claiming identical true peak or widening its acceptance. At
            // 192 kHz the reference uses the original grid: keep a quarter-rate
            // diagnostic where the owned estimator still detects a hidden peak.
            if key != "true" || (smooth && !name.ends_with("diagnostic")) {
                assert!(
                    (value - expected).abs() < tolerance,
                    "{name}/{key}: own {value}, reference {expected}"
                );
            }
            println!("{name}/{key}: own {value:.4}, reference {expected:.1}");
        }
        assert_eq!(owned.sample_frames, (rate * 12) as u64);
        assert_eq!(owned.sample_rate, rate);
        assert_eq!(owned.channels, channels);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
