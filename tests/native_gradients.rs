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
#[test]
fn masks_constants_and_invalid_input() {
    for kind in KINDS {
        for depth in [8, 10, 16] {
            let mut f = frame(depth);
            let original = f.data.clone();
            Gradient::parse(kind, "planes=0")
                .unwrap()
                .apply(&mut f, depth)
                .unwrap();
            assert_eq!(f.data, original);
            Gradient::parse(kind, "planes=1:scale=0:delta=42")
                .unwrap()
                .apply(&mut f, depth)
                .unwrap();
            let bytes = if depth == 8 { 1 } else { 2 };
            assert_eq!(&f.data[64 * bytes..], &original[64 * bytes..]);
            for sample in f.data[..64 * bytes].chunks_exact(bytes) {
                assert_eq!(sample[0], 42);
                if bytes == 2 {
                    assert_eq!(sample[1], 0);
                }
            }
            f.data.pop();
            let before = f.data.clone();
            assert!(
                Gradient::parse(kind, "")
                    .unwrap()
                    .apply(&mut f, depth)
                    .is_err()
            );
            assert_eq!(f.data, before);
        }
        for args in [
            "planes=16",
            "scale=NaN",
            "scale=-1",
            "delta=inf",
            "unknown=1",
            "1:1:0:1",
        ] {
            assert!(Gradient::parse(kind, args).is_err());
        }
        let mut flat = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![20, 30, 40],
        };
        Gradient::parse(kind, "")
            .unwrap()
            .apply(&mut flat, 8)
            .unwrap();
        assert_eq!(flat.data, [0, 0, 0]);
    }
}
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG; reference-only executable"]
fn pixels_match_reference() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let executable = std::env::var("FVID_REFERENCE_FFMPEG").unwrap();
    for (depth, format) in [(8, "yuv444p"), (10, "yuv444p10le"), (16, "yuv444p16le")] {
        for kind in KINDS {
            for args in ["", "planes=1:scale=0.125:delta=3"] {
                let mut f = frame(depth);
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
