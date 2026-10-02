//! Explicit reference benchmark; ordinary tests never invoke FFmpeg.
use fvid_media::owned_true_peak::TruePeakMeter;
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Instant,
};
fn main() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for (frequency, phase, channels) in [
        (1000., 0., 1),
        (12000., std::f64::consts::FRAC_PI_4, 1),
        (12000., std::f64::consts::FRAC_PI_4, 2),
    ] {
        let pcm: Vec<_> = (0usize..48000)
            .flat_map(|i| {
                let fade = (i.min(47999 - i) as f64 / 64.).min(1.);
                let sample = 0.5
                    * fade
                    * (std::f64::consts::TAU * frequency * i as f64 / 48000. + phase).sin();
                (0..channels).map(move |ch| if ch % 2 == 0 { sample } else { -sample })
            })
            .collect();
        let started = Instant::now();
        let mut meter = TruePeakMeter::new(channels).unwrap();
        for block in pcm.chunks(channels * 4096) {
            meter.push(block).unwrap();
        }
        meter.finish();
        let owned_time = started.elapsed();
        let owned = meter.report();
        let started = Instant::now();
        let mut process = Command::new(&ffmpeg)
            .args([
                "-nostdin", "-v", "info", "-f", "f64le", "-ar", "48000", "-ac",
            ])
            .arg(channels.to_string())
            .args([
                "-i",
                "pipe:0",
                "-af",
                "ebur128=peak=true:framelog=verbose",
                "-f",
                "null",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("reference benchmark requires FFmpeg");
        let mut input = process.stdin.take().unwrap();
        let mut bytes = Vec::with_capacity(channels * 4096 * 8);
        for block in pcm.chunks(channels * 4096) {
            bytes.clear();
            for sample in block {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            input.write_all(&bytes).unwrap();
        }
        drop(input);
        let output = process.wait_with_output().unwrap();
        let reference_time = started.elapsed();
        let log = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{log}");
        let expected: f64 = log
            .lines()
            .rev()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("Peak:")
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse().ok())
            })
            .expect("true-peak summary");
        let measured = owned.true_peak_dbfs.unwrap();
        assert!(
            (measured - expected).abs() < 0.2,
            "{frequency} Hz: owned {measured}, reference {expected}"
        );
        if frequency == 12000. {
            assert!(
                measured - owned.sample_peak_dbfs.unwrap() > 2.9,
                "intersample peak must be measured"
            );
        }
        println!("48 kHz/{channels}ch/{frequency}Hz: owned {measured:.4} dBTP ({owned_time:?}), FFmpeg {expected:.1} dBTP ({reference_time:?}; includes process startup)");
    }
}
