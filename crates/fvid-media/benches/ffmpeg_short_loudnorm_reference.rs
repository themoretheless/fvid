//! Explicit reference comparison for the dynamic filter's short-recording path.
use fvid_media::{
    CopyOptions, apply_loudnorm, owned_wav_file::write_wav_f64le, owned_wave_inspect::inspect,
};
use std::{process::Command, time::Instant};
fn main() {
    let dir = std::env::temp_dir().join(format!("fvid-short-reference-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let reference = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for (name, rate, channels, gain, args, tolerance) in [
        ("mono", 192000, 1, 0.01, "I=-16:TP=-1.5:LRA=11", 2e-6),
        (
            "dual",
            192000,
            1,
            0.01,
            "I=-16:TP=-1.5:LRA=11:dual_mono=true",
            2e-6,
        ),
        ("peak", 192000, 2, 0.8, "I=-5:TP=-3:LRA=11", 2e-6),
        (
            "stereo",
            192000,
            2,
            0.1,
            "I=-16:TP=-1.5:LRA=11:linear=false",
            2e-6,
        ),
        (
            "resample-48",
            48000,
            2,
            0.05,
            "I=-16:TP=-1.5:LRA=11",
            0.0001,
        ),
        (
            "resample-44",
            44100,
            1,
            0.05,
            "I=-16:TP=-1.5:LRA=11",
            0.0001,
        ),
        ("histogram", 192000, 1, 0.01, "I=-16:TP=-1.5:LRA=11", 2e-6),
    ] {
        let source = dir.join(format!("{name}.wav"));
        let output = dir.join(format!("{name}-out.wav"));
        let pcm: Vec<f64> = (0..rate)
            .flat_map(|i| {
                let fade = (i.min(rate - 1 - i) as f64 / 128.).min(1.);
                let sample =
                    gain * fade * (std::f64::consts::TAU * 1000. * i as f64 / rate as f64).sin();
                (0..channels).map(move |ch| if ch == 0 { sample } else { -sample })
            })
            .collect();
        if name == "histogram" {
            let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/playback-errors/loudnorm-histogram.wav");
            std::fs::copy(fixture, &source).unwrap();
        } else {
            write_wav_f64le(&source, rate, channels, &pcm).unwrap();
        }
        let start = Instant::now();
        let stats = apply_loudnorm(&source, &output, Some(args), &CopyOptions::default()).unwrap();
        let owned_time = start.elapsed();
        let start = Instant::now();
        let result = Command::new(&reference)
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(&source)
            .args([
                "-af",
                &format!("loudnorm={args},aformat=sample_fmts=flt"),
                "-f",
                "f32le",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let reference_time = start.elapsed();
        let mut file = std::fs::File::open(&output).unwrap();
        let info = inspect(&mut file, None).unwrap();
        let wave = std::fs::read(&output).unwrap();
        let owned =
            &wave[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize];
        assert_eq!(stats.sample_rate, 192000);
        assert_eq!(
            stats.sample_frames,
            if name == "histogram" { 96000 } else { 192000 }
        );
        assert_eq!(owned.len(), result.stdout.len(), "{name} frame count");
        let max_error = owned
            .chunks_exact(4)
            .zip(result.stdout.chunks_exact(4))
            .map(|(a, b)| {
                let a = f32::from_le_bytes(a.try_into().unwrap());
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!(a.is_finite() && b.is_finite());
                (a - b).abs()
            })
            .fold(0f32, f32::max);
        println!(
            "{name}: owned={owned_time:?}, reference with process startup={reference_time:?}, max PCM error={max_error}"
        );
        assert!(max_error <= tolerance, "{name}: {max_error} > {tolerance}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
