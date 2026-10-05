//! File loudness and normalization comparison benchmark.
mod file_tests {
    pub fn owned_file_decoders_feed_loudness_meter() {
        use std::process::Command;
        let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
        let dir = std::env::temp_dir().join(format!("fvid-loudness-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        for (index, (codec, ext)) in [
            ("pcm_s16le", "wav"),
            ("alac", "m4a"),
            ("aac", "m4a"),
            ("aac", "mka"),
            ("aac", "aac"),
        ]
        .iter()
        .enumerate()
        {
            let source = dir.join(format!("{index}.{ext}"));
            let result = Command::new(&binary)
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=1000:sample_rate=48000:duration=2",
                    "-c:a",
                    codec,
                ])
                .arg(&source)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let actual = fvid::native_pcm::measure_loudness_file(&source, None, &[1.0], None).unwrap();
            assert!(actual.sample_frames >= 96000);
            let normalized = dir.join(format!("{index}-normalized.wav"));
            let target = fvid::native_pcm::NormalizeTarget {
                integrated_lufs: -20.0,
                sample_peak_dbfs: -1.5,
            };
            let report = fvid::native_pcm::normalize_loudness_file(
                &source,
                &normalized,
                None,
                &[1.0],
                target,
                None,
            )
            .unwrap();
            assert!(!report.peak_limited);
            let decode = |path: &std::path::Path, gain: Option<f64>| {
                let mut command = Command::new(&binary);
                command.args(["-v", "error"]);
                if path.extension().and_then(|s| s.to_str()) == Some("f32le") {
                    command.args(["-f", "f32le", "-ar", "48000", "-ac", "1"]);
                }
                command.arg("-i").arg(path);
                if let Some(gain) = gain {
                    command.args(["-af", &format!("volume={gain}:precision=float")]);
                }
                let result = command.args(["-f", "f32le", "-"]).output().unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                result.stdout
            };
            let output = decode(&normalized, None);
            let baseline = dir.join(format!("{index}-decoded.f32le"));
            fvid::native_export::export_audio_pcm_selected(
                &source, &baseline, None, 1.0, None, None, None, None, None,
            )
            .unwrap();
            let reference = decode(&baseline, Some(10f64.powf(report.gain_db / 20.0)));
            if *codec == "aac" {
                let foreign = decode(&source, None);
                let owned = std::fs::read(&baseline).unwrap();
                assert_eq!(owned.len(), foreign.len());
                let mut maximum = 0f64;
                let mut squared = 0f64;
                for (a, b) in owned.as_chunks::<4>().0.iter().zip(foreign.as_chunks::<4>().0.iter()) {
                    let difference = f64::from(
                        f32::from_le_bytes(*a)
                            - f32::from_le_bytes(*b),
                    );
                    maximum = maximum.max(difference.abs());
                    squared += difference * difference;
                }
                eprintln!(
                    "AAC/{ext} decoder residual: max={maximum}, rms={}",
                    (squared / (owned.len() / 4) as f64).sqrt()
                );
            }
            assert_eq!(output.len(), reference.len(), "{codec}/{ext} sample count");
            for (i, (actual, expected)) in output
                .as_chunks::<4>().0.iter()
                .zip(reference.as_chunks::<4>().0.iter())
                .enumerate()
            {
                let actual = f32::from_le_bytes(*actual);
                let expected = f32::from_le_bytes(*expected);
                assert!(
                    (actual - expected).abs() < 1e-7,
                    "{codec}/{ext} sample {i}: {actual} != {expected}"
                );
            }

            assert!(fvid::native_pcm::measure_loudness_file(&source, None, &[1.0, 1.0], None).is_err());
            let cancel = fvid::media_control::CancelFlag::default();
            cancel.cancel();
            assert!(fvid::native_pcm::measure_loudness_file(&source, None, &[1.0], Some(&cancel)).is_err());
            let result = Command::new(&binary)
                .args(["-hide_banner", "-nostats", "-i"])
                .arg(&source)
                .args(["-af", "ebur128=peak=sample", "-f", "null", "-"])
                .output()
                .unwrap();
            assert!(result.status.success());
            let log = String::from_utf8(result.stderr).unwrap();
            let expected: f64 = log
                .lines()
                .rev()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("I:")
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap();
            assert!(
                (actual.integrated_lufs.unwrap() - expected).abs() < 0.11,
                "{codec}/{ext}: {actual:?} vs {expected}"
            );
            let expected_peak: f64 = log
                .lines()
                .rev()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("Peak:")
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap();
            assert!(
                (actual.sample_peak_dbfs.unwrap() - expected_peak).abs() < 0.11,
                "{codec}/{ext} peak: {:?} vs {expected_peak}",
                actual.sample_peak_dbfs
            );
        }
    }
}

fn main() {
 let start = std::time::Instant::now();
 file_tests::owned_file_decoders_feed_loudness_meter();
 println!("File loudness owned/reference validation: {:?}", start.elapsed());
}
