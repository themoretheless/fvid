//! Explicit independent filter reference benchmark.
use fvid::{native_geometry::GeometryFrame, native_pixelize::Pixelize};
fn main() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    for (layout, sub) in [
        ("420", [2, 2]),
        ("422", [2, 1]),
        ("444", [1, 1]),
        ("440", [1, 2]),
        ("411", [4, 1]),
        ("410", [4, 4]),
    ] {
        for depth in [8, 10, 16] {
            if (depth > 8 && matches!(layout, "411" | "410")) || (layout == "440" && depth == 16) {
                continue;
            }
            let n = 17 * 13 + 2 * 17usize.div_ceil(sub[0]) * 13usize.div_ceil(sub[1]);
            let input: Vec<u8> = (0..n)
                .flat_map(|i| {
                    let v = ((i * 173 + i * i * 11) % (1usize << depth)) as u16;
                    if depth == 8 {
                        vec![v as u8]
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            let format = if depth == 8 {
                format!("yuv{layout}p")
            } else {
                format!("yuv{layout}p{depth}le")
            };
            for args in [
                "",
                "3:5:avg:7",
                "w=5:h=3:m=min:p=5",
                "1:1:max:1",
                "1024:1024:2:7",
            ] {
                let mut frame = GeometryFrame {
                    width: 17,
                    height: 13,
                    subsampling: Some(sub),
                    data: input.clone(),
                };
                Pixelize::parse(args)
                    .unwrap()
                    .apply(&mut frame, depth)
                    .unwrap();
                let mut child = Command::new(&binary)
                    .args([
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        &format,
                        "-video_size",
                        "17x13",
                        "-i",
                        "pipe:0",
                        "-vf",
                        &format!("pixelize={args}"),
                        "-frames:v",
                        "1",
                        "-f",
                        "rawvideo",
                        "pipe:1",
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(&input).unwrap();
                let result = child.wait_with_output().unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(frame.data, result.stdout, "{format} {args}");
            }
        }
    }
}
