//! Explicit independent pixel reference benchmark.
use fvid::{
    native_geometry::GeometryFrame,
    native_pixels::{Gradient, GradientKind},
};
const KINDS: [GradientKind; 5] = [
    GradientKind::Sobel,
    GradientKind::Prewitt,
    GradientKind::Roberts,
    GradientKind::Kirsch,
    GradientKind::Scharr,
];
fn frame(depth: u8) -> GeometryFrame {
    let max = (1u32 << depth) - 1;
    let data = (0..192)
        .flat_map(|i| {
            let v = ((i * i * 31 + i * 17) % (max + 1)) as u16;
            if depth == 8 {
                vec![v as u8]
            } else {
                v.to_le_bytes().to_vec()
            }
        })
        .collect();
    GeometryFrame {
        width: 8,
        height: 8,
        subsampling: Some([1, 1]),
        data,
    }
}
fn main() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let executable = std::env::var("FVID_REFERENCE_FFMPEG").unwrap();
    for (depth, format, sx, sy) in [
        (8, "yuv444p", 1, 1),
        (10, "yuv444p10le", 1, 1),
        (16, "yuv444p16le", 1, 1),
        (8, "yuv422p", 2, 1),
        (10, "yuv422p10le", 2, 1),
        (16, "yuv422p16le", 2, 1),
        (8, "yuv420p", 2, 2),
        (10, "yuv420p10le", 2, 2),
        (16, "yuv420p16le", 2, 2),
    ] {
        for kind in KINDS {
            for args in ["", "planes=1:scale=0.125:delta=3",
                "planes=7/2:scale=default/8:delta=PI",
                "planes=max-8:scale=1/8:delta=default"] {
                let mut f = frame(depth);
                f.subsampling = Some([sx, sy]);
                f.data
                    .truncate((64 + 128 / (sx * sy)) * if depth == 8 { 1 } else { 2 });
                let data = f.data.clone();
                Gradient::parse(kind, args)
                    .unwrap()
                    .apply(&mut f, depth)
                    .unwrap();
                let filter = if args.is_empty() {
                    kind.name().to_owned()
                } else {
                    format!("{}={args}", kind.name())
                };
                let mut p = Command::new(&executable)
                    .args([
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        format,
                        "-video_size",
                        "8x8",
                        "-i",
                        "pipe:0",
                        "-vf",
                        &filter,
                        "-frames:v",
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
                p.stdin.take().unwrap().write_all(&data).unwrap();
                let out = p.wait_with_output().unwrap();
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                if let Some(index) = f.data.iter().zip(&out.stdout).position(|(a, b)| a != b) {
                    panic!(
                        "{format} {filter} byte {index}: own={} reference={}",
                        f.data[index], out.stdout[index]
                    );
                }
                assert_eq!(f.data.len(), out.stdout.len());
            }
        }
    }
}
