//! Explicit directional perspectiveing oracle; ordinary tests read saved synthetic pixels.
use fvid_media::owned_perspective::Perspective;
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let oracle =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit reference executable required");
    let mut count = 0;
    for (w, h) in [(17usize, 13usize), (1, 1)] {
        for depth in [8u8] {
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
                    "0:0:W:0:0:H:W:H:1:0:1",
                    "interpolation=0x1:sense=0:eval=1",
                    "x0=1:y0=2:x1=W-2:y1=1:x2=2:y2=H-1:x3=W-1:y3=H-2",
                    "x0=1:y0=2:x1=W-2:y1=1:x2=2:y2=H-1:x3=W-1:y3=H-2:interpolation=cubic",
                    "x0=-W/3:y0=-H/4:x1=W*1.25:y1=-H/8:x2=-W/4:y2=H*1.2:x3=W*1.1:y3=H*1.25",
                    "x0=2:y0=1:x1=W-1:y1=2:x2=1:y2=H-2:x3=W-2:y3=H-1:sense=destination",
                    "x0=2:y0=1:x1=W-1:y1=2:x2=1:y2=H-2:x3=W-2:y3=H-1:sense=destination:interpolation=cubic",
                    "x0=.5:y0=.25:x1=W+.5:y1=.25:x2=.5:y2=H+.25:x3=W+.5:y3=H+.25",
                    "x0=.5:y0=.25:x1=W+.5:y1=.25:x2=.5:y2=H+.25:x3=W+.5:y3=H+.25:interpolation=cubic",
                    "x0=in/3:y0=on/4:eval=frame",
                    "x0=in/3:y0=on/4:eval=init",
                    "x0=2:y0=1:enable='eq(n,2)'",
                ];
                for args in options {
                    if w == 1 && (args.contains("x1=W-2") || args.contains("sense=destination")) {
                        continue;
                    }
                    let filter = Perspective::parse(args).unwrap();
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
                            &format!("perspective={args}"),
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
                    let differences: Vec<_> = result
                        .stdout
                        .iter()
                        .zip(&expected)
                        .enumerate()
                        .filter(|(_, (a, b))| a != b)
                        .take(12)
                        .map(|(i, (a, b))| (i, *a, *b))
                        .collect();
                    assert!(
                        result.stdout == expected,
                        "{format} {w}x{h} {args}: {differences:?}, lengths {}/{}",
                        result.stdout.len(),
                        expected.len()
                    );
                    count += 1;
                    if w == 17
                        && h == 13
                        && std::env::var_os("FVID_WRITE_REFERENCE_FIXTURES").is_some()
                    {
                        let name = match (format.as_str(), args) {
                            ("yuv420p", "x0=1:y0=2:x1=W-2:y1=1:x2=2:y2=H-1:x3=W-1:y3=H-2") => {
                                Some("perspective-linear")
                            }
                            (
                                "yuv420p",
                                "x0=1:y0=2:x1=W-2:y1=1:x2=2:y2=H-1:x3=W-1:y3=H-2:interpolation=cubic",
                            ) => Some("perspective-cubic"),
                            (
                                "yuv420p",
                                "x0=2:y0=1:x1=W-1:y1=2:x2=1:y2=H-2:x3=W-2:y3=H-1:sense=destination",
                            ) => Some("perspective-destination"),
                            ("yuv420p", "x0=in/3:y0=on/4:eval=frame") => Some("perspective-frame"),
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
    println!("perspective: {count} exact four-frame comparisons passed");
}
