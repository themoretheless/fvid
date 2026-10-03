//! Explicit temporal compatibility and independent FFV1 decode reference.
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
    let source = root.join("shuffleframes-seven-frames.y4m");
    let directory =
        std::env::temp_dir().join(format!("fvid-shuffle-reference-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let ffmpeg = std::env::var_os("FVID_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for (case, mapping, interval, step, filter, expected, consumed) in [
        (
            "inverse",
            "2 1 0",
            None,
            None,
            "shuffleframes=2 1 0",
            vec![(2u8, 0u64), (1, 1), (0, 2), (5, 3), (4, 4), (3, 5)],
            7,
        ),
        (
            "drops",
            "mapping=2|-1|2",
            None,
            None,
            "shuffleframes=mapping=2|-1|2",
            vec![(2, 0), (2, 2), (5, 3), (5, 5)],
            7,
        ),
        (
            "range",
            "2 1 0",
            Some((250_000, 1_500_000)),
            None,
            "trim=start=0.25:end=1.5,shuffleframes=2 1 0",
            vec![(3, 1), (2, 2), (1, 3)],
            6,
        ),
        (
            "step",
            "2 1 0",
            None,
            Some("2"),
            "framestep=2,shuffleframes=2 1 0",
            vec![(4, 0), (2, 2), (0, 4)],
            7,
        ),
    ] {
        let transform = LosslessTransform {
            shuffleframes: Some(mapping.into()),
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
        if interval.is_none() {
            // A nonempty encoder option forces the preexisting adapter rather
            // than selecting the new own operation; level 1 is lossless too.
            let legacy = directory.join(format!("legacy-{case}.mkv"));
            let old = fvid_media::transcode(
                &source,
                &legacy,
                transform,
                &options,
                &fvid_media_info::EncoderSettings {
                    name: "ffv1".into(),
                    options: vec![("level".into(), "1".into())],
                },
            )
            .unwrap();
            assert_eq!(old.decoded_frames, consumed);
            assert_eq!(
                decode(&ffmpeg, &legacy),
                (pixels.clone(), timing.clone()),
                "previous adapter: {case}"
            );
        }
        println!(
            "{case}: {} pixels/timestamps match independent reference; {consumed} consumed",
            expected.len()
        );
    }
    #[cfg(feature = "legacy-ffmpeg")]
    for (case, mapping) in [("all-drop", "-1"), ("partial", "0 1 2 3 4 5 6 7")] {
        let output = directory.join(format!("legacy-empty-{case}.mkv"));
        let result = fvid_media::transcode(
            &source,
            &output,
            LosslessTransform {
                shuffleframes: Some(mapping.into()),
                ..Default::default()
            },
            &fvid_media::CopyOptions::default(),
            &fvid_media_info::EncoderSettings {
                name: "ffv1".into(),
                options: vec![("level".into(), "1".into())],
            },
        );
        assert!(result.unwrap_err().contains("no video frames decoded"));
    }
}
