//! Explicit reverse compatibility and independent FFV1 decode reference.
use fvid_media_info::LosslessTransform;
use std::{path::Path, process::Command};
fn decode(ffmpeg: &std::ffi::OsStr, path: &Path) -> (Vec<u8>, Vec<(f64, f64)>) {
    let result = Command::new(ffmpeg)
        .args(["-v", "info", "-threads", "1", "-i"])
        .arg(path)
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
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    let timing = stderr
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
    (result.stdout, timing)
}
fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    let source = root.join("reverse-six-frames.y4m");
    let directory =
        std::env::temp_dir().join(format!("fvid-reverse-reference-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let ffmpeg = std::env::var_os("FVID_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for (case, interval, step, shuffle, filter, expected, consumed) in [
        (
            "full",
            None,
            None,
            None,
            "reverse",
            vec![(5u8, 0u64), (4, 1), (3, 2), (2, 3), (1, 4), (0, 5)],
            6,
        ),
        (
            "range",
            Some((250_000, 1_000_000)),
            None,
            None,
            "trim=start=0.25:end=1,reverse",
            vec![(3, 1), (2, 2), (1, 3)],
            4,
        ),
        (
            "step",
            None,
            Some("2"),
            None,
            "framestep=2,reverse",
            vec![(4, 0), (2, 2), (0, 4)],
            6,
        ),
        (
            "shuffle",
            None,
            None,
            Some("2 1 0"),
            "shuffleframes=2 1 0,reverse",
            vec![(3, 0), (4, 1), (5, 2), (0, 3), (1, 4), (2, 5)],
            6,
        ),
    ] {
        let transform = LosslessTransform {
            reverse: Some(String::new()),
            shuffleframes: shuffle.map(String::from),
            interval,
            framestep: step.map(String::from),
            ..Default::default()
        };
        let options = fvid_media::CopyOptions::default();
        let output = directory.join(format!("{case}.mkv"));
        let stats =
            fvid_media::transcode_lossless(&source, &output, transform.clone(), &options).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.decoded_frames, consumed);
        assert_eq!(stats.video_frames, expected.len() as u64);
        let (pixels, timing) = decode(&ffmpeg, &output);
        let expected_pixels: Vec<_> = expected
            .iter()
            .flat_map(|&(value, _)| [vec![10 + value; 16], vec![128; 8]].concat())
            .collect();
        assert_eq!(pixels, expected_pixels, "independent pixels: {case}");
        let origin = interval.map_or(0.0, |(from, _)| from as f64 / 1_000_000.0);
        assert_eq!(
            timing,
            expected
                .iter()
                .map(|&(_, position)| (position as f64 / 4.0 - origin, 0.25))
                .collect::<Vec<_>>(),
            "independent timing: {case}"
        );
        let reference = Command::new(&ffmpeg)
            .args(["-v", "error", "-threads", "1", "-i"])
            .arg(&source)
            .args([
                "-vf",
                filter,
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
        assert_eq!(pixels, reference.stdout, "reference filter: {case}");
        #[cfg(feature = "legacy-ffmpeg")]
        if case == "full" {
            // Additional external level-1 oracle. FVid still encodes its owned
            // format; this comparison makes no native level-1 support claim.
            let encoded = directory.join(format!("reference-level1-{case}.mkv"));
            let reference = Command::new(&ffmpeg)
                .args(["-v", "error", "-threads", "1", "-i"])
                .arg(&source)
                .args([
                    "-vf",
                    filter,
                    "-fps_mode",
                    "passthrough",
                    "-c:v",
                    "ffv1",
                    "-level",
                    "1",
                ])
                .arg(&encoded)
                .output()
                .unwrap();
            assert!(
                reference.status.success(),
                "{}",
                String::from_utf8_lossy(&reference.stderr)
            );
            let (reference_pixels, reference_timing) = decode(&ffmpeg, &encoded);
            assert_eq!(reference_pixels, pixels);
            // FFmpeg derives Matroska DefaultDuration from the filtered nominal
            // rate (framestep can change it). Owned durations remain checked
            // against the independent original-clock expectations above.
            assert_eq!(
                reference_timing
                    .iter()
                    .map(|event| event.0)
                    .collect::<Vec<_>>(),
                timing.iter().map(|event| event.0).collect::<Vec<_>>()
            );
        }
        println!(
            "{case}: {} pixels/timestamps match independent reference; {consumed} consumed",
            expected.len()
        );
    }
}
