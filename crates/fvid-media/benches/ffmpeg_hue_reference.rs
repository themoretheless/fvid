//! Explicit reference benchmark; ordinary tests and production never spawn codecs.
use std::{path::Path, process::Command};
fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    for depth in [8, 10] {
        let source = root.join(format!("hue-{depth}.y4m"));
        let bytes = std::fs::read(&source).unwrap();
        let size = if depth == 8 { 6 } else { 12 };
        for args in [
            "",
            "h=90",
            "h=-45:s=1.2:b=0.2",
            "s=0:b=1",
            "H=1.25:s=-2:b=-0.5",
        ] {
            let mut frame = fvid_media::owned_frame::GeometryFrame {
                width: 2,
                height: 2,
                subsampling: Some([2, 2]),
                data: bytes[bytes.len() - size..].to_vec(),
            };
            fvid_media::owned_hue::Hue::parse(args)
                .unwrap()
                .apply(&mut frame, depth)
                .unwrap();
            let reference =
                Command::new(std::env::var("FVID_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()))
                    .args(["-v", "error", "-i"])
                    .arg(&source)
                    .args([
                        "-vf",
                        &format!("hue={args}"),
                        "-frames:v",
                        "1",
                        "-pix_fmt",
                        if depth == 8 { "yuv420p" } else { "yuv420p10le" },
                        "-f",
                        "rawvideo",
                        "pipe:1",
                    ])
                    .output()
                    .unwrap();
            assert!(
                reference.status.success(),
                "{}",
                String::from_utf8_lossy(&reference.stderr)
            );
            assert_eq!(frame.data, reference.stdout, "depth={depth}; hue={args}");
            println!("{depth}-bit hue={args}: exact pixels match reference");
        }
    }
}
