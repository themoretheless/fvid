//! Explicit drawing oracle; ordinary tests read independently qualified synthetic bytes.
use fvid_media::{
    owned_draw::{Draw, Kind},
    owned_frame::GeometryFrame,
};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let oracle =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit reference executable required");
    if std::env::var_os("FVID_DRAW_CHAIN_ONLY").is_some() {
        qualify_combined(&oracle);
        return;
    }
    let mut count = 0;
    for kind in [Kind::Box, Kind::Grid] {
        let name = if matches!(kind, Kind::Box) {
            "drawbox"
        } else {
            "drawgrid"
        };
        for (w, h) in [(17usize, 13usize), (16, 12), (1, 1)] {
            for (format, sx, sy, alpha, packed) in [
                ("yuv420p", 2, 2, false, false),
                ("yuv422p", 2, 1, false, false),
                ("yuv444p", 1, 1, false, false),
                ("yuv440p", 1, 2, false, false),
                ("yuv411p", 4, 1, false, false),
                ("yuv410p", 4, 4, false, false),
                ("yuvj444p", 1, 1, false, false),
                ("yuva420p", 2, 2, true, false),
                ("yuva422p", 2, 1, true, false),
                ("yuva444p", 1, 1, true, false),
                ("rgb24", 1, 1, false, true),
                ("rgba", 1, 1, true, true),
            ] {
                let size = if packed {
                    w * h * if alpha { 4 } else { 3 }
                } else {
                    w * h + 2 * w.div_ceil(sx) * h.div_ceil(sy) + if alpha { w * h } else { 0 }
                };
                for options in [
                    "",
                    "x=1:y=2:w=9:h=7:c=red@.4:t=2",
                    "-3:-2:9:7:blue@.5:3",
                    "x=iw/2-w/2:y=ih/2-h/2:w=iw/3:h=ih/2:c=#12345678:t=1",
                    "x=fill:y=fill:w=0:h=0:t=fill:c=white",
                    "x=1:y=2:w=fill:h=fill:c=lime:t=fill",
                    "x=3:y=4:w=5:h=6:c=invert:t=2",
                    "x=1:y=2:w=9:h=7:c=red@.4:t=2:replace=1",
                    "w=4:h=3:t=0",
                    "w=-2:h=-3:t=1",
                    "w=4:h=3:t=-1",
                    "w=4:h=3:t=20:c=yellow",
                    "w=5:h=4:t=2:c=white@0",
                    "w=5:h=4:t=1:enable='eq(n,2)'",
                    "x=sar:y=dar:w=iw/hsub:h=ih/vsub:t=1",
                ] {
                    // RGB has zero chroma subsampling exponents; this expression deliberately divides by them.
                    if options.contains("hsub") && (sx == 1 || sy == 1) {
                        continue;
                    }
                    let filter = Draw::parse(kind, options).unwrap();
                    let mut input = Vec::new();
                    let mut expected = Vec::new();
                    for n in 0..4 {
                        let data: Vec<_> = (0..size)
                            .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
                            .collect();
                        input.extend_from_slice(&data);
                        let mut data = data;
                        if packed && alpha {
                            filter
                                .apply_rgba(&mut data, w, h, n as u64, Some(n as f64 / 25.))
                                .unwrap();
                        } else if alpha {
                            filter
                                .apply_yuva(
                                    &mut data,
                                    w,
                                    h,
                                    [sx, sy],
                                    8,
                                    1.,
                                    n as u64,
                                    Some(n as f64 / 25.),
                                )
                                .unwrap();
                        } else {
                            let mut frame = GeometryFrame {
                                width: w,
                                height: h,
                                subsampling: if packed { None } else { Some([sx, sy]) },
                                data,
                            };
                            filter
                                .apply(&mut frame, 8, n as u64, Some(n as f64 / 25.))
                                .unwrap();
                            data = frame.data;
                        }
                        expected.extend(data);
                    }
                    let output =
                        reference(&oracle, format, w, h, &format!("{name}={options}"), input);
                    assert_eq!(output, expected, "{name} {format} {w}x{h} {options}");
                    count += 1;
                    if w == 17
                        && h == 13
                        && format == "yuv420p"
                        && options == "x=1:y=2:w=9:h=7:c=red@.4:t=2"
                        && std::env::var_os("FVID_WRITE_REFERENCE_FIXTURES").is_some()
                    {
                        std::fs::write(
                            format!("tests/fixtures/playback-errors/{name}.expected.raw"),
                            output,
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
    println!("drawing: {count} exact four-frame comparisons passed");
    qualify_aspect(&oracle);
    qualify_combined(&oracle);
}
fn reference(
    oracle: &std::ffi::OsStr,
    format: &str,
    w: usize,
    h: usize,
    filter: &str,
    input: Vec<u8>,
) -> Vec<u8> {
    let mut child = Command::new(oracle)
        .args([
            "-v",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            format,
            "-video_size",
            &format!("{w}x{h}"),
            "-framerate",
            "25",
            "-i",
            "pipe:0",
            "-vf",
            filter,
            "-threads",
            "1",
            "-frames:v",
            "4",
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
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let result = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
    assert!(
        result.status.success(),
        "{filter}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
fn qualify_aspect(oracle: &std::ffi::OsStr) {
    for kind in [Kind::Box, Kind::Grid] {
        let name = if matches!(kind, Kind::Box) {
            "drawbox"
        } else {
            "drawgrid"
        };
        let args = "x=sar*3:y=dar:w=iw/2:h=ih/2:c=red@.4:t=2";
        let filter = Draw::parse(kind, args).unwrap();
        let mut input = Vec::new();
        let mut expected = Vec::new();
        for n in 0..4 {
            let data: Vec<_> = (0..347)
                .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
                .collect();
            input.extend_from_slice(&data);
            let mut frame = GeometryFrame {
                width: 17,
                height: 13,
                subsampling: Some([2, 2]),
                data,
            };
            filter
                .apply_with_aspect(&mut frame, 8, 4. / 3., n as u64, Some(n as f64 / 25.))
                .unwrap();
            expected.extend(frame.data);
        }
        let output = reference(
            oracle,
            "yuv420p",
            17,
            13,
            &format!("setsar=4/3,{name}={args}"),
            input,
        );
        assert_eq!(output, expected);
        if std::env::var_os("FVID_WRITE_REFERENCE_FIXTURES").is_some() {
            std::fs::write(
                format!("tests/fixtures/playback-errors/{name}-aspect.expected.raw"),
                output,
            )
            .unwrap();
        }
    }
    println!("drawing: 2 non-square pixel aspect comparisons passed");
}

fn qualify_combined(oracle: &std::ffi::OsStr) {
    let first = Draw::box_filter("x=1:y=2:w=9:h=7:c=red@.4:t=2").unwrap();
    let second = Draw::grid_filter("w=5:h=4:c=blue@.3:t=1").unwrap();
    let mut input = Vec::new();
    let mut expected = Vec::new();
    let mut reverse = Vec::new();
    for n in 0..4 {
        let data: Vec<_> = (0..347)
            .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
            .collect();
        input.extend_from_slice(&data);
        let mut a = GeometryFrame {
            width: 17,
            height: 13,
            subsampling: Some([2, 2]),
            data: data.clone(),
        };
        let mut b = GeometryFrame {
            width: 17,
            height: 13,
            subsampling: Some([2, 2]),
            data,
        };
        first
            .apply(&mut a, 8, n as u64, Some(n as f64 / 25.))
            .unwrap();
        second
            .apply(&mut a, 8, n as u64, Some(n as f64 / 25.))
            .unwrap();
        expected.extend(a.data);
        second
            .apply(&mut b, 8, n as u64, Some(n as f64 / 25.))
            .unwrap();
        first
            .apply(&mut b, 8, n as u64, Some(n as f64 / 25.))
            .unwrap();
        reverse.extend(b.data);
    }
    assert_ne!(
        expected, reverse,
        "synthetic input must distinguish compositing order"
    );
    let output = reference(
        oracle,
        "yuv420p",
        17,
        13,
        "drawbox=x=1:y=2:w=9:h=7:c=red@.4:t=2,drawgrid=w=5:h=4:c=blue@.3:t=1",
        input,
    );
    assert_eq!(output, expected);
    if std::env::var_os("FVID_WRITE_REFERENCE_FIXTURES").is_some() {
        std::fs::write(
            "tests/fixtures/playback-errors/drawing-combined.expected.raw",
            output,
        )
        .unwrap();
    }
    println!("drawing: box/grid compositing order qualified");
}
