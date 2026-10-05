//! Explicit directional debanding comparison; ordinary tests use portable synthetic pixels.
//! Random sampling maps can differ from FFmpeg's platform binary32 libm math.
use fvid_media::owned_deband::Deband;
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let oracle =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit reference executable required");
    let mut count = 0;
    let mut portable_map_differences = 0;
    for (w, h) in [(17usize, 13usize), (1, 1)] {
        for depth in [8u8, 9, 10, 12, 14, 16] {
            for (base, sx, sy, alpha, packed) in [
                ("yuv420p", 2, 2, false, false),
                ("yuv422p", 2, 1, false, false),
                ("yuv444p", 1, 1, false, false),
                ("yuv440p", 1, 2, false, false),
                ("yuv411p", 4, 1, false, false),
                ("yuv410p", 4, 4, false, false),
                ("yuva444p", 1, 1, true, false),
                ("gbrp", 1, 1, false, false),
                ("gbrap", 1, 1, true, false),
                ("gray", 1, 1, false, false),
                ("rgb24", 1, 1, false, true),
                ("rgba", 1, 1, true, true),
            ] {
                if depth != 8 && (packed || matches!(base, "yuv440p" | "yuv411p" | "yuv410p"))
                    || base == "gbrap" && depth != 8 && depth != 16
                    || base == "yuva444p" && matches!(depth, 12 | 14)
                {
                    continue;
                }
                let format = if depth == 8 {
                    base.into()
                } else if base == "gray" {
                    format!("gray{depth}le")
                } else {
                    format!("{base}{depth}le")
                };
                let dims = if base == "gray" {
                    vec![(w, h)]
                } else {
                    vec![
                        (w, h),
                        (w.div_ceil(sx), h.div_ceil(sy)),
                        (w.div_ceil(sx), h.div_ceil(sy)),
                    ]
                };
                let mut dims = dims;
                if alpha {
                    dims.push((w, h));
                }
                let options = [
                    "",
                    "1thr=.5:2thr=.5:3thr=.5:4thr=.5",
                    "1thr=.5:2thr=.5:3thr=.5:4thr=.5:b=0",
                    "1thr=.2:2thr=.3:3thr=.4:4thr=.1:r=-4:d=-PI/2",
                    "r=4:d=PI/2",
                    "r=0",
                    "r=-65535:d=0:1thr=.5:2thr=.5:3thr=.5:4thr=.5",
                    "r=65535:d=0",
                    "r=4:d=-2*PI",
                    "r=-4:d=0:1thr=.00003:2thr=.00003:3thr=.00003:4thr=.00003",
                    "enable='eq(n,2)'",
                    "enable='eq(w,17)*eq(h,13)'",
                    "1thr=.5:2thr=.5:3thr=.5:4thr=.5:c=1",
                    "1thr=.5:2thr=.5:3thr=.5:4thr=.5:c=1:b=0",
                ];
                for args in options {
                    let filter = Deband::parse(args).unwrap();
                    if filter.coupled() && (sx != 1 || sy != 1 || base == "gray") {
                        continue;
                    }
                    let mut input = Vec::new();
                    let mut expected = Vec::new();
                    for n in 0..4 {
                        let mut data = Vec::new();
                        if packed {
                            for y in 0..h {
                                for x in 0..w {
                                    for p in 0..dims.len() {
                                        data.push(
                                            ((x * 7 + y * 11 + p * 53 + n * 13) % 192 + 32) as u8,
                                        );
                                    }
                                }
                            }
                        } else {
                            for (p, (pw, ph)) in dims.iter().copied().enumerate() {
                                for y in 0..ph {
                                    for x in 0..pw {
                                        let v = (((x * 7 + y * 11 + p * 53 + n * 13) % 192 + 32)
                                            as u16)
                                            << (depth - 8);
                                        if depth == 8 {
                                            data.push(v as u8);
                                        } else {
                                            data.extend(v.to_le_bytes());
                                        }
                                    }
                                }
                            }
                        }
                        input.extend_from_slice(&data);
                        if packed {
                            filter
                                .apply_rgb(
                                    &mut data,
                                    w,
                                    h,
                                    depth,
                                    dims.len(),
                                    n as u64,
                                    Some(n as f64 / 25.),
                                )
                                .unwrap();
                        } else if base == "gray" {
                            filter
                                .apply_gray(&mut data, w, h, depth, n as u64, Some(n as f64 / 25.))
                                .unwrap();
                        } else {
                            filter
                                .apply_planar(
                                    &mut data,
                                    w,
                                    h,
                                    [sx, sy],
                                    depth,
                                    alpha,
                                    n as u64,
                                    Some(n as f64 / 25.),
                                )
                                .unwrap();
                        }
                        expected.extend(data);
                    }
                    let mut child = Command::new(&oracle)
                        .args([
                            "-v",
                            "error",
                            "-f",
                            "rawvideo",
                            "-pixel_format",
                            &format,
                            "-video_size",
                            &format!("{w}x{h}"),
                            "-framerate",
                            "25",
                            "-i",
                            "pipe:0",
                            "-vf",
                            &format!("deband={args}"),
                            "-threads",
                            "1",
                            "-frames:v",
                            "4",
                            "-f",
                            "rawvideo",
                            "-pix_fmt",
                            &format,
                            "pipe:1",
                        ])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .unwrap();
                    let mut stdin = child.stdin.take().unwrap();
                    let writer = std::thread::spawn(move || stdin.write_all(&input));
                    let result = child.wait_with_output().unwrap();
                    writer.join().unwrap().unwrap();
                    assert!(
                        result.status.success(),
                        "{format} {args}: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                    assert_eq!(
                        result.stdout.len(),
                        expected.len(),
                        "{format} {w}x{h} {args}"
                    );
                    if result.stdout != expected {
                        // A fixed radius and direction do not use the random
                        // map; their pixel semantics must still match exactly.
                        if args == "r=0"
                            || args.contains("r=-")
                                && (args.contains("d=-") || args.contains("d=0"))
                        {
                            assert_eq!(result.stdout, expected, "{format} {w}x{h} {args}");
                        }
                        let different_bytes = result
                            .stdout
                            .iter()
                            .zip(&expected)
                            .filter(|(reference, owned)| reference != owned)
                            .count();
                        println!(
                            "portable sampling map difference: {format} {w}x{h} {args:?}: {different_bytes}/{} bytes",
                            expected.len()
                        );
                        portable_map_differences += 1;
                    }
                    count += 1;
                    if w == 17
                        && h == 13
                        && std::env::var_os("FVID_WRITE_REFERENCE_FIXTURES").is_some()
                    {
                        let name = match (format.as_str(), args) {
                            ("yuv420p", "") => Some("deband-default"),
                            ("yuv420p", "1thr=.5:2thr=.5:3thr=.5:4thr=.5") => Some("deband-strong"),
                            ("yuv420p", "1thr=.5:2thr=.5:3thr=.5:4thr=.5:b=0") => {
                                Some("deband-no-blur")
                            }
                            ("yuv444p", "1thr=.5:2thr=.5:3thr=.5:4thr=.5:c=1") => {
                                Some("deband-coupled")
                            }
                            ("yuv444p16le", "1thr=.5:2thr=.5:3thr=.5:4thr=.5:c=1") => {
                                Some("deband-depth")
                            }
                            _ => None,
                        };
                        if let Some(name) = name {
                            std::fs::write(
                                format!("tests/fixtures/playback-errors/{name}.expected.raw"),
                                result.stdout,
                            )
                            .unwrap();
                        }
                    }
                }
            }
        }
    }
    println!(
        "deband: {count} four-frame comparisons; {} exact matches, {portable_map_differences} portable sampling map differences",
        count - portable_map_differences
    );
}
