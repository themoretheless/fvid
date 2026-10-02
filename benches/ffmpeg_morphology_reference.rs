//! Explicit independent pixel reference benchmark.
use fvid::{
    native_geometry::GeometryFrame,
    native_morphology::{Morphology, MorphologyKind},
};
const KINDS: [MorphologyKind; 2] = [MorphologyKind::Dilation, MorphologyKind::Erosion];
fn frame(depth: u8, sx: usize, sy: usize, rgb: bool) -> GeometryFrame {
    let max = (1u32 << depth) - 1;
    let count = if rgb { 192 } else { 64 + 128 / (sx * sy) };
    let data = (0..count)
        .flat_map(|i| {
            let v = ((i * i * 31 + i * 17) as u32 % (max + 1)) as u16;
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
        subsampling: if rgb { None } else { Some([sx, sy]) },
        data,
    }
}
fn main() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let executable = std::env::var("FVID_REFERENCE_FFMPEG").unwrap();
    for (depth, format, sx, sy, rgb) in [
        (8, "yuv444p", 1, 1, false),
        (10, "yuv444p10le", 1, 1, false),
        (16, "yuv444p16le", 1, 1, false),
        (8, "yuv422p", 2, 1, false),
        (10, "yuv422p10le", 2, 1, false),
        (16, "yuv422p16le", 2, 1, false),
        (8, "yuv420p", 2, 2, false),
        (10, "yuv420p10le", 2, 2, false),
        (16, "yuv420p16le", 2, 2, false),
        (8, "rgb24", 1, 1, true),
    ] {
        for kind in KINDS {
            for args in [
                "",
                "coordinates=1",
                "coordinates=2",
                "coordinates=4",
                "coordinates=8",
                "coordinates=16",
                "coordinates=32",
                "coordinates=64",
                "coordinates=128",
                "threshold0=3:threshold1=0:threshold2=17",
                "170:11:5:0:0",
            ] {
                let mut f = frame(depth, sx, sy, rgb);
                let data = f.data.clone();
                Morphology::parse(kind, args)
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
                if let Some(i) = f.data.iter().zip(&out.stdout).position(|(a, b)| a != b) {
                    panic!(
                        "{format} {filter} byte {i}: own={} ref={}",
                        f.data[i], out.stdout[i]
                    );
                }
                assert_eq!(f.data.len(), out.stdout.len());
            }
        }
    }
}
