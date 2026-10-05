//! Explicit reference qualification; ordinary tests consume saved synthetic pixels.
use fvid_media::{owned_frame::GeometryFrame, owned_removegrain::RemoveGrain};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let oracle = std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit reference required");
    qualify_chain(&oracle);
    let mut count = 0;
    for (w, h) in [(17usize, 13usize), (16, 12), (3, 3), (1, 1)] {
        for (format, sx, sy, planes, packed) in [
            ("yuv420p", 2, 2, 3, false),
            ("yuv422p", 2, 1, 3, false),
            ("yuv444p", 1, 1, 3, false),
            ("yuv440p", 1, 2, 3, false),
            ("yuv411p", 4, 1, 3, false),
            ("yuv410p", 4, 4, 3, false),
            ("yuva420p", 2, 2, 4, false),
            ("gbrp", 1, 1, 3, false),
            ("gbrap", 1, 1, 4, false),
            ("gray", 1, 1, 1, false),
            ("rgb24", 1, 1, 3, true),
        ] {
            let dims: Vec<_> = (0..planes)
                .map(|p| {
                    if format.starts_with("yuv") && (p == 1 || p == 2) {
                        (w.div_ceil(sx), h.div_ceil(sy))
                    } else {
                        (w, h)
                    }
                })
                .collect();
            let size: usize = dims.iter().map(|(w, h)| w * h).sum();
            let options = (0..=24)
                .map(|m| format!("m0={m}:m1={m}:m2={m}:m3={m}"))
                .chain(
                    [
                        "m0=13:m1=14:m2=19",
                        "m0=1:m1=13:m2=20",
                        "m0=24:m1=5:m2=10",
                        "m3=19",
                        "m0=19:m3=13",
                        "m0=19:enable='eq(n,2)'",
                        "m0=20:enable='eq(w,17)*eq(h,13)'",
                        "0x13:19:19:0",
                    ]
                    .map(str::to_string),
                );
            for args in options {
                let filter = RemoveGrain::parse(&args).unwrap();
                let mut input = Vec::new();
                let mut expected = Vec::new();
                for n in 0..4 {
                    let data: Vec<_> = (0..size)
                        .map(|i| ((i * i * 13 + i * 37 + n * 23 + 101) % 256) as u8)
                        .collect();
                    input.extend(&data);
                    let mut out = data;
                    if packed {
                        let mut frame = GeometryFrame {
                            width: w,
                            height: h,
                            subsampling: None,
                            data: out,
                        };
                        filter
                            .apply(&mut frame, 8, n as u64, Some(n as f64 / 25.))
                            .unwrap();
                        out = frame.data;
                    } else {
                        let mut offset = 0;
                        for (plane, (pw, ph)) in dims.iter().copied().enumerate() {
                            filter
                                .apply_plane_in_frame(
                                    &mut out[offset..offset + pw * ph],
                                    pw,
                                    ph,
                                    8,
                                    plane,
                                    [w, h],
                                    planes,
                                    n as u64,
                                    Some(n as f64 / 25.),
                                )
                                .unwrap();
                            offset += pw * ph;
                        }
                    }
                    expected.extend(out);
                }
                let mut command = Command::new(&oracle);
                command
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
                        &format!("removegrain={args}"),
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
                    .stderr(Stdio::piped());
                let mut child = command.spawn().unwrap();
                let mut stdin = child.stdin.take().unwrap();
                let writer = std::thread::spawn(move || stdin.write_all(&input));
                let result = child.wait_with_output().unwrap();
                writer.join().unwrap().unwrap();
                assert!(
                    result.status.success(),
                    "{format} {w}x{h} {args}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(result.stdout, expected, "{format} {w}x{h} {args}");
                count += 1;
                if w == 17
                    && h == 13
                    && format == "yuv420p"
                    && std::env::var_os("FVID_WRITE_REFERENCE_FIXTURES").is_some()
                    && let Some(mode) = args
                        .strip_prefix("m0=")
                        .and_then(|v| v.split(':').next())
                        .and_then(|v| v.parse::<u8>().ok())
                    && args == format!("m0={mode}:m1={mode}:m2={mode}:m3={mode}")
                {
                    std::fs::write(format!("tests/fixtures/playback-errors/removegrain-{mode:02}.expected.raw"),result.stdout).unwrap();
                }
            }
        }
    }
    println!("removegrain: {count} exact four-frame comparisons passed");
}

fn qualify_chain(oracle: &std::ffi::OsStr) {
    let pixelize = fvid_media::owned_pixelize::Pixelize::parse("3:3:avg:7").unwrap();
    let grain = RemoveGrain::parse("20:19:10").unwrap();
    let blur = fvid_media::owned_yaepblur::YaepBlur::parse("r=2:p=7:s=1024").unwrap();
    let mut input = Vec::new();
    let mut expected = Vec::new();
    let mut reversed = Vec::new();
    for n in 0..4 {
        let data: Vec<_> = (0..347)
            .map(|i| ((i * i * 13 + i * 37 + n * 23 + 101) % 256) as u8)
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
        pixelize.apply(&mut a, 8).unwrap();
        grain
            .apply(&mut a, 8, n as u64, Some(n as f64 / 25.))
            .unwrap();
        blur.apply(&mut a, 8, n as u64, Some(n as f64 / 25.))
            .unwrap();
        expected.extend(a.data);
        grain
            .apply(&mut b, 8, n as u64, Some(n as f64 / 25.))
            .unwrap();
        pixelize.apply(&mut b, 8).unwrap();
        blur.apply(&mut b, 8, n as u64, Some(n as f64 / 25.))
            .unwrap();
        reversed.extend(b.data);
    }
    assert_ne!(expected, reversed, "chain fixture must distinguish order");
    let mut child = Command::new(oracle)
        .args([
            "-v",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            "yuv420p",
            "-video_size",
            "17x13",
            "-framerate",
            "25",
            "-i",
            "pipe:0",
            "-vf",
            "pixelize=3:3:avg:7,removegrain=20:19:10,yaepblur=r=2:p=7:s=1024",
            "-threads",
            "1",
            "-frames:v",
            "4",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
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
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, expected);
    if std::env::var_os("FVID_WRITE_REFERENCE_FIXTURES").is_some() {
        std::fs::write(
            "tests/fixtures/playback-errors/removegrain-chain.expected.raw",
            result.stdout,
        )
        .unwrap();
    }
    println!("removegrain: pixelize/removegrain/yaepblur chain order qualified");
}
