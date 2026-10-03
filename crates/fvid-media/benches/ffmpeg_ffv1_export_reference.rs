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

    let source = root.join("ffv1-six-frames.mkv");
    for (case, reverse, shuffle, step, interval, filter) in [
        ("reverse", true, None, None, None, "reverse"),
        (
            "range",
            true,
            None,
            None,
            Some((40_000, 200_000)),
            "trim=start=0.04:end=0.20,setpts=PTS-0.04/TB,reverse",
        ),
        (
            "compose",
            true,
            Some("2|1|0".into()),
            Some("2".into()),
            None,
            "framestep=2,shuffleframes=mapping=2|1|0,reverse",
        ),
        (
            "shuffle",
            false,
            Some("2|0|1".into()),
            None,
            None,
            "shuffleframes=mapping=2|0|1",
        ),
        (
            "tail",
            true,
            Some("2|1|0".into()),
            None,
            Some((40_000, 240_000)),
            "trim=start=0.04:end=0.24,setpts=PTS-0.04/TB,shuffleframes=mapping=2|1|0,reverse",
        ),
        (
            "drops",
            true,
            Some("2|-1|2".into()),
            None,
            None,
            "shuffleframes=mapping=2|-1|2,reverse",
        ),
    ] {
        let output = std::env::temp_dir().join(format!(
            "fvid-ffv1-temporal-reference-{}-{case}.mkv",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        fvid_media::transcode_lossless(
            &source,
            &output,
            LosslessTransform {
                reverse: reverse.then(String::new),
                shuffleframes: shuffle,
                framestep: step,
                interval,
                ..Default::default()
            },
            &CopyOptions::default(),
        )
        .unwrap();
        let read = |path: &Path, filter: Option<&str>| {
            let mut command = Command::new(&ffmpeg);
            command
                .args(["-hide_banner", "-nostdin", "-v", "info", "-i"])
                .arg(path);
            let graph = filter.map_or("showinfo".into(), |f| format!("{f},showinfo"));
            let result = command
                .args([
                    "-map",
                    "0:v:0",
                    "-vf",
                    &graph,
                    "-fps_mode",
                    "passthrough",
                    "-pix_fmt",
                    "gray",
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
            let log = String::from_utf8_lossy(&result.stderr);
            let times: Vec<f64> = log
                .lines()
                .filter_map(|line| {
                    line.split_once("pts_time:")
                        .and_then(|(_, s)| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .collect();
            (result.stdout, times)
        };
        let actual = read(&output, None);
        let reference = read(&source, Some(filter));
        assert_eq!(actual, reference, "temporal pixels and positions: {case}");
        std::fs::remove_file(output).unwrap();
    }

    let output = std::env::temp_dir().join(format!(
        "fvid-ffv1-tags-reference-{}.mkv",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&output);
    fvid_media::transcode_lossless(
        &root.join("ffv1-track-tags.mkv"),
        &output,
        Default::default(),
        &CopyOptions {
            metadata_set: vec![("title".into(), "New title".into())],
            ..Default::default()
        },
    )
    .unwrap();
    let result = Command::new(&ffmpeg)
        .args(["-hide_banner", "-nostdin", "-v", "info", "-i"])
        .arg(&output)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let log = String::from_utf8_lossy(&result.stderr);
    for (key, value) in [
        ("FVID_TEST_NOTE", "own container metadata"),
        ("TITLE", "New title"),
        ("ENCODER", "synthetic source"),
        ("PRIVATE_TRACK_NOTE", "not file metadata"),
    ] {
        assert!(
            log.lines().any(|line| line
                .trim()
                .split_once(':')
                .is_some_and(
                    |(name, text)| name.trim().eq_ignore_ascii_case(key) && text.trim() == value
                )),
            "missing tag {key}={value}: {log}"
        );
    }
    std::fs::remove_file(output).unwrap();

    for (case, interval, step, reverse, tail) in [
        ("full", None, None, false, ""),
        (
            "range",
            Some((250_000, 1_000_000)),
            None,
            false,
            ",trim=start=0.25:end=1,setpts=PTS-0.25/TB",
        ),
        ("step", None, Some("2".into()), false, ",framestep=2"),
        ("reverse", None, None, true, ",reverse"),
    ] {
        let output = std::env::temp_dir().join(format!(
            "fvid-ffv1-overlay-reference-{}-{case}.mkv",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        let source = root.join("ffv1-overlay-vfr.mkv");
        let secondary = root.join("overlay-secondary-clock.y4m");
        fvid_media::transcode_lossless(
            &source,
            &output,
            LosslessTransform {
                overlay: Some(fvid_media::OverlaySpec {
                    path: secondary.clone(),
                    x: 3,
                    y: 3,
                }),
                interval,
                framestep: step,
                reverse: reverse.then(String::new),
                ..Default::default()
            },
            &CopyOptions::default(),
        )
        .unwrap();
        let read = |owned: bool| {
            let mut command = Command::new(&ffmpeg);
            command
                .args(["-hide_banner", "-nostdin", "-copyts", "-v", "info", "-i"])
                .arg(if owned { &output } else { &source });
            if owned {
                command.args(["-vf", "showinfo"]);
            } else {
                command
                    .arg("-i")
                    .arg(&secondary)
                    .arg("-filter_complex")
                    .arg(format!(
                        "[0:v][1:v]overlay=x=3:y=3:format=yuv420{tail},showinfo[out]"
                    ))
                    .args(["-map", "[out]"]);
            }
            let result = command
                .args([
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
            let log = String::from_utf8_lossy(&result.stderr);
            let times: Vec<f64> = log
                .lines()
                .filter_map(|line| {
                    line.split_once("pts_time:")
                        .and_then(|(_, s)| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .collect();
            (result.stdout, times)
        };
        assert_eq!(
            read(true),
            read(false),
            "overlay pixels and presentation times: {case}"
        );
        std::fs::remove_file(output).unwrap();
    }
    println!(
        "owned FFV1 export: 20 pixel/timing references and independent file tag checks passed"
    );
}
