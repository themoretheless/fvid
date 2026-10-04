//! Explicit benchmark oracle only; production and ordinary tests never execute FFmpeg.
use fvid_media::{owned_fade::Fade, owned_frame::GeometryFrame};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let executable =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit benchmark oracle required");
    let mut cases = 0;
    for (format, sub, depth, full) in [
        ("rgb24", None, 8, true),
        ("yuv420p", Some([2, 2]), 8, false),
        ("yuvj420p", Some([2, 2]), 8, true),
        ("yuv444p", Some([1, 1]), 8, false),
        ("yuv420p10le", Some([2, 2]), 10, false),
        ("yuv444p12le", Some([1, 1]), 12, false),
        ("yuv444p16le", Some([1, 1]), 16, false),
    ] {
        for args in [
            "in:0:2",
            "out:1:2",
            "out:0:3",
            "in:2:3",
            "out:2:4",
            "out:1:65537",
        ] {
            let mut input = Vec::new();
            let mut expected = Vec::new();
            for n in 0..7 {
                let samples = if let Some([sx, sy]) = sub {
                    15 + 2 * 5usize.div_ceil(sx) * 3usize.div_ceil(sy)
                } else {
                    45
                };
                let mut data = Vec::new();
                for i in 0..samples {
                    let value = ((i * 173 + n * 31) % (1usize << depth)) as u16;
                    if depth == 8 {
                        data.push(value as u8);
                    } else {
                        data.extend(value.to_le_bytes());
                    }
                }
                input.extend(&data);
                let mut frame = GeometryFrame {
                    width: 5,
                    height: 3,
                    subsampling: sub,
                    data,
                };
                Fade::parse(args)
                    .unwrap()
                    .apply(&mut frame, depth, full, n as u64)
                    .unwrap();
                expected.extend(frame.data);
            }
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
                    &format!("fade={args}"),
                    "-frames:v",
                    "7",
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
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, expected, "{format} {args}");
            cases += 1;
        }
    }

    for (format, channels) in [("rgb24", 3usize), ("rgba", 4usize)] {
        for color in [
            "red",
            "blue",
            "green",
            "lime",
            "white",
            "orange",
            "0x123456",
            "0x123456ff",
            "red@1",
        ] {
            for direction in ["in", "out"] {
                let args = format!("{direction}:1:3:color={color}");
                let filter = Fade::parse(&args).unwrap();
                let mut input = Vec::new();
                let mut expected = Vec::new();
                for n in 0..7 {
                    let mut bytes = (0..15 * channels)
                        .map(|i| ((i * 173 + n * 31) % 256) as u8)
                        .collect::<Vec<_>>();
                    input.extend(&bytes);
                    filter.apply_rgb(&mut bytes, 8, channels, n as u64).unwrap();
                    expected.extend(bytes);
                }
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
                        &format!("fade={args}"),
                        "-frames:v",
                        "7",
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
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(output.stdout, expected, "{format} {args}");
                cases += 1;
            }
        }
    }


    for (rate, num, den) in [("25", 25u64, 1u64), ("30000/1001", 30000, 1001)] {
        for format in ["rgb24", "yuv420p", "yuv420p10le"] {
            for args in [
                "in:d=0.1",
                "out:st=0.1:d=0.1",
                "out:s=1:d=0.1",
                "in:s=2:st=0.05:d=0.12",
                "out:st=0.1:n=2",
                "in:d=0.000001",
                "out:s=1:st=0.02:n=3",
                "out:s=1:d=100ms",
                "out:st=100000us:d=0.1s",
                "in:st=50.9999ms:d=120.9999ms",
                "in:d=0.1000009",
                "out:d=1.9us",
            ] {
                let fade = fvid_media::owned_fade::FadeClock::parse(args).unwrap();
                let (sub, depth) = if format == "rgb24" {
                    (None, 8)
                } else if format == "yuv420p" {
                    (Some([2, 2]), 8)
                } else {
                    (Some([2, 2]), 10)
                };
                let mut input = Vec::new();
                let mut expected = Vec::new();
                for n in 0..9 {
                    let samples = if sub.is_some() { 27 } else { 45 };
                    let mut data = Vec::new();
                    for i in 0..samples {
                        let value = ((i * 173 + n * 31) % (1usize << depth)) as u16;
                        if depth == 8 {
                            data.push(value as u8);
                        } else {
                            data.extend(value.to_le_bytes());
                        }
                    }
                    input.extend(&data);
                    let mut frame = GeometryFrame {
                        width: 5,
                        height: 3,
                        subsampling: sub,
                        data,
                    };
                    let time = fvid_media::owned_fade::FrameTime::new(n as u128 * den as u128, num)
                        .unwrap()
                        .with_quantum(den)
                        .unwrap();
                    fade.at(n as u64, Some(time))
                        .unwrap()
                        .apply(&mut frame, depth, sub.is_none(), n as u64)
                        .unwrap();
                    expected.extend(frame.data);
                }
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
                        rate,
                        "-i",
                        "pipe:0",
                        "-vf",
                        &format!("fade={args}"),
                        "-frames:v",
                        "9",
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
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(output.stdout, expected, "{format} {rate} {args}");
                cases += 1;
            }
        }
    }
    println!("fade: {cases} exact pixel comparisons passed");
}
