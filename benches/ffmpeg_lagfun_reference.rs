//! Explicit benchmark oracle only; never executed in ordinary tests or production.
use fvid_media::{owned_frame::GeometryFrame, owned_lagfun::LagFun};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let executable =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit benchmark oracle required");
    let mut cases = 0;
    for (format, sub, depth) in [
        ("yuv420p", [2, 2], 8),
        ("yuv444p", [1, 1], 8),
        ("yuv420p10le", [2, 2], 10),
        ("yuv444p10le", [1, 1], 10),
        ("yuv420p16le", [2, 2], 16),
        ("yuv444p16le", [1, 1], 16),
    ] {
        for args in [
            "",
            "decay=0.5",
            "decay=1",
            "decay=0",
            "decay=0.95:planes=1",
            "decay=0.7:planes=2",
            "decay=0.8:enable='between(n,2,4)'",
        ] {
            let filter = LagFun::parse(args).unwrap();
            let mut input = Vec::new();
            let mut expected = Vec::new();
            for n in 0..8usize {
                let count = 15 + 2 * 5usize.div_ceil(sub[0]) * 3usize.div_ceil(sub[1]);
                let mut data = Vec::new();
                for i in 0..count {
                    let v = if n % 3 == 0 {
                        (i * 173 + n * 31) % (1usize << depth)
                    } else {
                        (i * 17) % 128
                    } as u16;
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
                "lagfun".into()
            } else {
                format!("lagfun={args}")
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
            assert_eq!(out.stdout, expected, "{format} {args}");
            cases += 1;
        }
    }
    println!("lagfun: {cases} exact pixel comparisons passed");
}
