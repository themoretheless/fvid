//! Explicit external reference for the owned temporal file-export operation.
use fvid_media_info::LosslessTransform;
use std::{path::Path, process::Command};
fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    let source = root.join("framestep-six-frames.y4m");
    let directory =
        std::env::temp_dir().join(format!("fvid-framestep-reference-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let ffmpeg = std::env::var_os("FVID_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for (case, interval, indices, consumed) in [
        ("full", None, vec![0u8, 2, 4], 6),
        ("range", Some((250_000, 1_000_000)), vec![1u8, 3], 4),
    ] {
        let output = directory.join(format!("{case}.mkv"));
        let stats = fvid_media::transcode_lossless(
            &source,
            &output,
            LosslessTransform {
                framestep: Some("2".into()),
                interval,
                ..Default::default()
            },
            &fvid_media::CopyOptions::default(),
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(
            (stats.decoded_frames, stats.video_frames),
            (consumed, indices.len() as u64)
        );
        let expected: Vec<u8> = indices
            .iter()
            .flat_map(|&i| [vec![10 + i; 16], vec![128; 8]].concat())
            .collect();
        let decoded = Command::new(&ffmpeg)
            .args(["-v", "info", "-threads", "1", "-i"])
            .arg(&output)
            .args([
                "-vf",
                "showinfo",
                "-fps_mode",
                "passthrough",
                "-pix_fmt",
                "yuv420p",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert_eq!(
            decoded.stdout, expected,
            "independent FFV1 decoding: {case}"
        );
        let stderr = String::from_utf8_lossy(&decoded.stderr);
        let timing: Vec<_> = stderr
            .lines()
            .filter_map(|line| {
                let pts = line
                    .split_once(" pts_time:")?
                    .1
                    .split_whitespace()
                    .next()?
                    .parse::<f64>()
                    .ok()?;
                let duration = line
                    .split_once(" duration_time:")?
                    .1
                    .split_whitespace()
                    .next()?
                    .parse::<f64>()
                    .ok()?;
                Some((pts, duration))
            })
            .collect();
        let origin = interval.map_or(0.0, |(from, _)| from as f64 / 1_000_000.0);
        let expected_timing: Vec<_> = indices
            .iter()
            .map(|&i| (f64::from(i) / 4.0 - origin, 0.25))
            .collect();
        assert_eq!(
            timing, expected_timing,
            "independent container timing: {case}"
        );
        let reference = Command::new(&ffmpeg)
            .args(["-v", "error", "-threads", "1", "-i"])
            .arg(&source)
            .args([
                "-vf",
                if interval.is_some() {
                    "trim=start=0.25:end=1,framestep=2"
                } else {
                    "framestep=2"
                },
                "-fps_mode",
                "passthrough",
                "-pix_fmt",
                "yuv420p",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            reference.status.success(),
            "{}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(
            decoded.stdout, reference.stdout,
            "temporal selection: {case}"
        );
        println!(
            "{case}: {consumed} consumed, {} output frames; independently decoded pixels match reference",
            indices.len()
        );
    }
}
