//! Explicit independent filter reference benchmark.
use fvid::{native_geometry::GeometryFrame, native_pixels::Negate};

fn frame(data: Vec<u8>, rgb: bool) -> GeometryFrame {
    GeometryFrame {
        width: 2,
        height: 2,
        subsampling: if rgb { None } else { Some([2, 2]) },
        data,
    }
}

fn main() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let ffmpeg = std::env::var("FVID_REFERENCE_FFMPEG").expect("set reference executable");
    for (format, rgb, depth) in [
        ("rgb24", true, 8),
        ("yuv420p", false, 8),
        ("yuv420p10le", false, 10),
        ("yuv420p16le", false, 16),
    ] {
        let data = if rgb {
            vec![0, 10, 255, 1, 20, 128, 2, 30, 127, 3, 40, 64]
        } else if depth == 8 {
            vec![16, 235, 0, 255, 128, 240]
        } else {
            [0u16, 64, 128, 511, 512, 1023]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect()
        };
        let mut own = frame(data.clone(), rgb);
        Negate.apply(&mut own, depth).unwrap();
        for filter in ["negate", "negate=negate_alpha=1"] {
            let mut child = Command::new(&ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-f",
                    "rawvideo",
                    "-pixel_format",
                    format,
                    "-video_size",
                    "2x2",
                    "-i",
                    "pipe:0",
                    "-vf",
                    filter,
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
            child.stdin.take().unwrap().write_all(&data).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(own.data, out.stdout, "{format} {filter}");
        }
    }
}
