//! Explicit FFmpeg comparison benchmark; never part of ordinary tests.
mod reference_tests {
    use fvid_media::owned_k_weight::KWeighting;
    pub fn agrees_with_independent_48khz_biquad_filter() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let pcm: Vec<f64> = (0..8192)
            .map(|i| {
                if i == 0 {
                    1.0
                } else {
                    (i as f64 * 0.13).sin() * 0.2
                }
            })
            .collect();
        let mut expected = Command::new(std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap())
            .args(["-v", "error", "-f", "f64le", "-ar", "48000", "-ac", "1", "-i", "pipe:0", "-af",
                "biquad=b0=1.53512485958697:b1=-2.69169618940638:b2=1.19839281085285:a0=1:a1=-1.69065929318241:a2=0.73248077421585:precision=f64,biquad=b0=1:b1=-2:b2=1:a0=1:a1=-1.99004745483398:a2=0.99007225036621:precision=f64",
                "-f", "f64le", "pipe:1"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let bytes: Vec<u8> = pcm.iter().flat_map(|x| x.to_le_bytes()).collect();
        let mut input = expected.stdin.take().unwrap();
        let writer = std::thread::spawn(move || input.write_all(&bytes).unwrap());
        let result = expected.wait_with_output().unwrap();
        writer.join().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut actual = pcm;
        KWeighting::new(48000, 1)
            .unwrap()
            .process(&mut actual)
            .unwrap();
        assert_eq!(result.stdout.len(), actual.len() * 8);
        for (i, (actual, expected)) in actual.iter().zip(result.stdout.chunks_exact(8)).enumerate()
        {
            let expected = f64::from_le_bytes(expected.try_into().unwrap());
            assert!(
                (actual - expected).abs() < 1e-8,
                "sample {i}: {actual} != {expected}"
            );
        }
    }
}
mod oracle {
    use fvid_media::owned_loudness::LoudnessMeter;
    pub fn integrated_loudness_matches_independent_meter() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        for rate in [44100, 48000, 96000] {
            let pcm: Vec<f64> = (0..rate * 18)
                .map(|i| {
                    let gain = if i < rate * 3 {
                        0.0
                    } else if i < rate * 9 {
                        0.1
                    } else {
                        0.02
                    };
                    gain * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / rate as f64).sin()
                })
                .collect();
            let mut meter = LoudnessMeter::new(rate, &[1.0]).unwrap();
            for part in pcm.chunks(337) {
                meter.push(part).unwrap();
            }
            let mut child = Command::new(std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap())
                .args([
                    "-hide_banner",
                    "-nostats",
                    "-f",
                    "f64le",
                    "-ar",
                    &rate.to_string(),
                    "-ac",
                    "1",
                    "-i",
                    "pipe:0",
                    "-af",
                    "ebur128",
                    "-f",
                    "null",
                    "-",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut input = child.stdin.take().unwrap();
            let writer = std::thread::spawn(move || {
                for sample in pcm {
                    input.write_all(&sample.to_le_bytes()).unwrap();
                }
            });
            let result = child.wait_with_output().unwrap();
            writer.join().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
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
            let report = meter.report();
            let actual = report.integrated_lufs.unwrap();
            assert!(
                (actual - expected).abs() < 0.11,
                "{rate}: {actual} != {expected}"
            );
            let range: f64 = log
                .lines()
                .rev()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("LRA:")
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap();
            assert!(
                (report.range_lu.unwrap() - range).abs() < 0.2,
                "{rate} LRA: {:?} != {range}",
                report.range_lu
            );
        }
    }
}

fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("set FVID_REFERENCE_FFMPEG for this comparison benchmark");
    let start = std::time::Instant::now();
    reference_tests::agrees_with_independent_48khz_biquad_filter();
    println!("K-weighting owned/reference validation: {:?}", start.elapsed());
    let start = std::time::Instant::now();
    oracle::integrated_loudness_matches_independent_meter();
    println!("Integrated loudness owned/reference validation: {:?}", start.elapsed());
}
