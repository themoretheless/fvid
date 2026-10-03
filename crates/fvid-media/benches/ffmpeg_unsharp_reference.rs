//! Explicit optional reference benchmark; never called by ordinary tests.
use std::{path::Path, process::Command};
fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    for depth in [8, 10, 16] {
        let source = root.join(format!("unsharp-{depth}.y4m"));
        let input = std::fs::read(&source).unwrap();
        let size = 192 * if depth == 8 { 1 } else { 2 };
        for args in [
            "",
            "3:3:1:3:3:0.5",
            "7:5:-1:5:7:-0.5",
            "23:3:0.2:3:23:0.5",
            "3:3:0:3:3:0",
        ] {
            let mut frame = fvid_media::owned_frame::GeometryFrame {
                width: 8,
                height: 8,
                subsampling: Some([1, 1]),
                data: input[input.len() - size..].to_vec(),
            };
            fvid_media::owned_unsharp::Unsharp::parse(args)
                .unwrap()
                .apply(&mut frame, depth)
                .unwrap();
            let format = if depth == 8 {
                "yuv444p".into()
            } else {
                format!("yuv444p{depth}le")
            };
            let result =
                Command::new(std::env::var("FVID_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()))
                    .args(["-v", "error", "-filter_threads", "1", "-i"])
                    .arg(&source)
                    .args([
                        "-vf",
                        &format!("unsharp={args}"),
                        "-frames:v",
                        "1",
                        "-pix_fmt",
                        &format,
                        "-f",
                        "rawvideo",
                        "pipe:1",
                    ])
                    .output()
                    .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(frame.data, result.stdout, "depth={depth}; unsharp={args}");
            println!("{depth}-bit unsharp={args}: exact pixels match reference");
        }
    }
}
