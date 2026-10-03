//! Explicit independent reference only; ordinary tests never invoke FFmpeg.
use fvid_media::{CopyOptions, LosslessTransform};
use std::{path::Path, process::Command};
fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    let ffmpeg = std::env::var_os("FVID_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for depth in [8, 10, 16] {
        let mut cases = vec![
            ("full", None, None, vec![0, 1], vec![0.0, 0.04]),
            ("step", None, Some("2".into()), vec![0], vec![0.0]),
            (
                "range",
                Some((40_000, 80_000)),
                Some("2".into()),
                vec![1],
                vec![0.0],
            ),
        ];
        if depth == 8 {
            cases.push(("vfr", None, None, vec![0, 1], vec![0.0, 0.073]));
        }
        for (case, interval, step, indices, times) in cases {
            let output = std::env::temp_dir().join(format!(
                "fvid-ffv1-reference-{}-{depth}-{case}.mkv",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&output);
            let source = if case == "vfr" {
                root.join("ffv1-vfr.mkv")
            } else {
                root.join(format!("ffv1-gray-{depth}.mkv"))
            };
            let transform = LosslessTransform {
                horizontal_flip: true,
                interval,
                framestep: step,
                ..Default::default()
            };
            fvid_media::transcode_lossless(&source, &output, transform, &CopyOptions::default())
                .unwrap();
            let format = if depth == 8 {
                "gray".into()
            } else {
                format!("gray{depth}le")
            };
            let result = Command::new(&ffmpeg)
                .args(["-hide_banner", "-nostdin", "-v", "info", "-i"])
                .arg(&output)
                .args([
                    "-map",
                    "0:v:0",
                    "-vf",
                    "showinfo",
                    "-fps_mode",
                    "passthrough",
                    "-pix_fmt",
                    &format,
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ])
                .output()
                .expect("explicit reference benchmark requires FFmpeg");
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let bytes = if depth == 8 { 1 } else { 2 };
            let expected: Vec<u8> = indices
                .iter()
                .flat_map(|index| {
                    let raw = std::fs::read(root.join(format!("ffv1-gray-{depth}-{index}.gray")))
                        .unwrap();
                    raw.chunks_exact(4 * bytes)
                        .flat_map(|row| row.chunks_exact(bytes).rev().flatten().copied())
                        .collect::<Vec<_>>()
                })
                .collect();
            assert_eq!(result.stdout, expected, "{depth} {case}");
            let log = String::from_utf8_lossy(&result.stderr);
            let actual: Vec<f64> = log
                .lines()
                .filter_map(|line| {
                    line.split_once("pts_time:")
                        .and_then(|(_, s)| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .collect();
            assert_eq!(actual, times, "timestamps: {depth} {case}");
            std::fs::remove_file(output).unwrap();
        }
    }
    println!("owned FFV1 export: 10 independent pixel and presentation-time references passed");
}
