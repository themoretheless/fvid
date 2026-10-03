//! Explicit compatibility benchmark; no production codec process.
use std::{path::Path, process::Command};
fn main() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/eq-ramp.y4m");
    let bytes = std::fs::read(&source).unwrap();
    for args in [
        "",
        "contrast=0:saturation=0",
        "brightness=0.06:contrast=1.2",
        "contrast=-2:brightness=-0.25:saturation=2.5",
        "gamma=2.2:gamma_weight=0.5",
        "gamma_r=1.5:gamma_g=0.8:gamma_b=2",
        "contrast=8:brightness=0.1",
        "contrast=1000:brightness=1",
        "1.2:0.1:0.5:1.5",
    ] {
        let mut frame = fvid_media::owned_frame::GeometryFrame {
            width: 16,
            height: 16,
            subsampling: Some([1, 1]),
            data: bytes[bytes.len() - 768..].to_vec(),
        };
        fvid_media::owned_eq::Equalizer::parse(args)
            .unwrap()
            .apply(&mut frame, 8)
            .unwrap();
        let output = Command::new(std::env::var("FVID_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()))
            .args(["-v", "error", "-i"])
            .arg(&source)
            .args([
                "-vf",
                &format!("eq={args}"),
                "-frames:v",
                "1",
                "-pix_fmt",
                "yuv444p",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(frame.data, output.stdout, "eq={args}");
        println!("eq={args}: all 256 values in each plane match reference");
    }
}
