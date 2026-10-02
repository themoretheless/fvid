//! Explicit target/performance comparison, not a bit-equivalence test.
//! FVid uses its own offline controller and linked limiter; reference metrics
//! describe how the different dynamic algorithms behave on the same signals.
use fvid_media::{
    CopyOptions, apply_loudnorm, owned_wav_file::write_wav_f64le,
    owned_wave_loudness::measure_loudness,
};
use std::{process::Command, time::Instant};
fn main() {
    let dir = std::env::temp_dir().join(format!("fvid-dynamic-reference-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for (name, seconds, channels) in [("constant", 6, 1), ("range", 12, 2), ("peaks", 6, 1)] {
        let rate = 192000;
        let source = dir.join(format!("{name}.wav"));
        let owned_file = dir.join(format!("{name}-owned.wav"));
        let reference_file = dir.join(format!("{name}-reference.wav"));
        let pcm: Vec<f64> = (0..rate * seconds)
            .flat_map(|i| {
                let gain = if name == "range" {
                    if i < rate * 4 {
                        0.003
                    } else if i < rate * 8 {
                        0.3
                    } else {
                        0.03
                    }
                } else {
                    0.03
                };
                let value = if name == "peaks" && i % rate == rate / 2 {
                    0.9
                } else {
                    gain * (std::f64::consts::TAU * 1000. * i as f64 / rate as f64).sin()
                };
                (0..channels).map(move |ch| if ch == 0 { value } else { -value / 2. })
            })
            .collect();
        write_wav_f64le(&source, rate, channels, &pcm).unwrap();
        let start = Instant::now();
        let stats = apply_loudnorm(
            &source,
            &owned_file,
            Some("I=-16:TP=-1.5:LRA=11"),
            &CopyOptions::default(),
        )
        .unwrap();
        let owned_time = start.elapsed();
        let start = Instant::now();
        let result = Command::new(&ffmpeg)
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(&source)
            .args([
                "-af",
                "loudnorm=I=-16:TP=-1.5:LRA=11,aformat=sample_fmts=flt",
                "-c:a",
                "pcm_f32le",
            ])
            .arg(&reference_file)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let reference_time = start.elapsed();
        let input = measure_loudness(&source, &CopyOptions::default()).unwrap();
        let owned = measure_loudness(&owned_file, &CopyOptions::default()).unwrap();
        let reference = measure_loudness(&reference_file, &CopyOptions::default()).unwrap();
        assert_eq!(stats.backend, "fvid dynamic loudnorm");
        assert_eq!(stats.sample_frames, (seconds * rate) as u64);
        assert_eq!(owned.sample_frames, reference.sample_frames);
        assert_eq!(stats.channels, channels);
        assert_eq!(stats.sample_rate, rate);
        assert!(
            owned.true_peak_dbfs <= -1.5 + 1e-5,
            "{name}: {}",
            owned.true_peak_dbfs
        );
        assert!(
            (owned.integrated_lufs + 16.).abs() < 0.3,
            "{name}: {}",
            owned.integrated_lufs
        );
        if name == "range" {
            assert!(owned.range_lu <= 12. && owned.range_lu < input.range_lu - 3.);
        }
        println!(
            "{name}: owned={owned_time:?}, reference including process startup={reference_time:?}"
        );
        println!(
            "input I={:.4} LRA={:.4} TP={:.4}; owned I={:.4} LRA={:.4} TP={:.4}; reference I={:.4} LRA={:.4} TP={:.4}",
            input.integrated_lufs,
            input.range_lu,
            input.true_peak_dbfs,
            owned.integrated_lufs,
            owned.range_lu,
            owned.true_peak_dbfs,
            reference.integrated_lufs,
            reference.range_lu,
            reference.true_peak_dbfs
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
