//! Explicit benchmark oracle only; never executed in ordinary tests or production.
use fvid_media::{owned_frame::GeometryFrame, owned_hqdn3d::HqDn3d};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let executable =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit benchmark oracle required");
    let mut cases = 0;
    let mut overflows = 0;
    for (format, sub, depth) in [
        ("yuv420p", [2, 2], 8),
        ("yuv444p", [1, 1], 8),
        ("yuv420p9le", [2, 2], 9),
        ("yuv420p10le", [2, 2], 10),
        ("yuv420p12le", [2, 2], 12),
        ("yuv420p14le", [2, 2], 14),
        ("yuv422p", [2, 1], 8),
        ("yuv440p", [1, 2], 8),
        ("yuv411p", [4, 1], 8),
        ("yuv410p", [4, 4], 8),
        ("yuv444p10le", [1, 1], 10),
        ("yuv420p16le", [2, 2], 16),
        ("yuv444p16le", [1, 1], 16),
    ] {
        for extreme in [false, true] {
            for args in [
                "",
                "0:0:0:0",
                "1:2:3:4",
                "20:15:30:25",
                "252:252:252:252",
                "luma_spatial=2:chroma_tmp=9",
                "luma_spatial=5.2:chroma_spatial=1.1:luma_tmp=7.8",
                "4:3:6:4.5:enable='between(n,2,4)'",
            ] {
                let filter = HqDn3d::parse(args).unwrap();
                let mut input = Vec::new();
                let mut expected = Vec::new();
                for n in 0..8usize {
                    let count = 15 + 2 * 5usize.div_ceil(sub[0]) * 3usize.div_ceil(sub[1]);
                    let mut data = Vec::new();
                    for i in 0..count {
                        let v = if extreme && n % 3 == 0 {
                            if i % 2 == n % 2 {
                                ((1u32 << depth) - 1) as u16
                            } else {
                                0
                            }
                        } else {
                            (((i * 7 + n * 3) % 32 + 90) << (depth - 8)) as u16
                        };
                        if depth == 8 {
                            data.push(v as u8);
                        } else {
                            data.extend(v.to_le_bytes());
                        }
                    }
                    input.extend(&data);
                    let mut frame = GeometryFrame {
                        width: 5,
                        height: 3,
                        subsampling: Some(sub),
                        data,
                    };
                    filter
                        .apply(&mut frame, depth, n as u64, Some(n as f64 / 25.))
                        .unwrap();
                    expected.extend(frame.data);
                }
                let graph = if args.is_empty() {
                    "hqdn3d".into()
                } else {
                    format!("hqdn3d={args}")
                };
                let mut child = Command::new(&executable)
                    .args([
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        format,
                        "-video_size",
                        "5x3",
                        "-framerate",
                        "25",
                        "-i",
                        "pipe:0",
                        "-vf",
                        &graph,
                        "-frames:v",
                        "8",
                        "-threads",
                        "1",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        format,
                        "pipe:1",
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(&input).unwrap();
                let out = child.wait_with_output().unwrap();
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                assert_eq!(out.stdout.len(), expected.len());
                let maximum = (1u32 << depth) - 1;
                let invalid = depth > 8
                    && out
                        .stdout
                        .chunks_exact(2)
                        .any(|v| u32::from(u16::from_le_bytes([v[0], v[1]])) > maximum);
                assert!(
                    depth == 8
                        || expected
                            .chunks_exact(2)
                            .all(|v| u32::from(u16::from_le_bytes([v[0], v[1]])) <= maximum)
                );
                if invalid {
                    assert!(extreme, "unexpected reference overflow in moderate grid");
                    assert_eq!(format, "yuv420p14le");
                    overflows += 1;
                    if format == "yuv420p14le" && args.is_empty() {
                        std::fs::write("/tmp/fvid-hqdn3d-extreme14-reference.raw", &out.stdout)
                            .unwrap();
                        std::fs::write("/tmp/fvid-hqdn3d-extreme14-owned.raw", &expected).unwrap();
                        assert_eq!(u16::from_le_bytes([expected[0], expected[1]]), 16383);
                    }
                } else {
                    assert_eq!(out.stdout, expected, "{format} {args} extreme={extreme}");
                    cases += 1;
                }
                if !extreme && format == "yuv420p" && args == "20:15:30:25" {
                    std::fs::write("/tmp/fvid-hqdn3d-grid-reference.raw", &out.stdout).unwrap();
                }
            }
        }
    }
    uniform_reference(&executable);
    cases += 2;
    println!(
        "hqdn3d: {cases} exact pixel comparisons passed; {overflows} reference sample overflows reproduced"
    );
}

fn uniform_reference(executable: &std::ffi::OsStr) {
    for (depth, format) in [(8, "yuv420p"), (10, "yuv420p10le")] {
        let filter = HqDn3d::parse("").unwrap();
        let mut input = Vec::new();
        let mut expected = Vec::new();
        for (n, delta) in [0i32, -4, 4, -4, 4, -4, 4, 0].into_iter().enumerate() {
            let mut data = Vec::new();
            for (base, count) in [(100, 16), (140, 4), (200, 4)] {
                let v = ((base + delta) << (depth - 8)) as u16;
                for _ in 0..count {
                    if depth == 8 {
                        data.push(v as u8);
                    } else {
                        data.extend(v.to_le_bytes());
                    }
                }
            }
            input.extend(&data);
            let mut frame = GeometryFrame {
                width: 4,
                height: 4,
                subsampling: Some([2, 2]),
                data,
            };
            filter
                .apply(&mut frame, depth, n as u64, Some(n as f64 / 25.))
                .unwrap();
            expected.extend(frame.data);
        }
        let mut child = Command::new(executable)
            .args([
                "-v",
                "error",
                "-f",
                "rawvideo",
                "-pixel_format",
                format,
                "-video_size",
                "4x4",
                "-framerate",
                "25",
                "-i",
                "pipe:0",
                "-vf",
                "hqdn3d",
                "-frames:v",
                "8",
                "-threads",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                format,
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&input).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, expected, "uniform {depth}");
        std::fs::write(
            format!("/tmp/fvid-hqdn3d-uniform-{depth}-reference.raw"),
            &out.stdout,
        )
        .unwrap();
    }
}
