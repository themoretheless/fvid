//! Explicit external reference for synthetic Matroska laces, never ordinary tests.
use std::{path::Path, process::Command};
fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    let ffmpeg = std::env::var_os("FVID_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for name in [
        "matroska-lace-xiph.mkv",
        "matroska-lace-ebml.mkv",
        "matroska-lace-fixed.mkv",
        "matroska-lace-group--5000000.mkv",
        "matroska-lace-group-5000000.mkv",
    ] {
        let source = root.join(name);
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            std::fs::File::open(&source).unwrap(),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 3, 1 << 20).unwrap();
        let mut expected = Vec::new();
        for i in 0..reader.packets.len() {
            let packet = reader.read_packet(i).unwrap();
            let decoded = decoder.decode(&packet).unwrap();
            expected.extend_from_slice(&decoded.frame.data[..12]);
        }
        let reference = Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&source)
            .args([
                "-map",
                "0:v:0",
                "-pix_fmt",
                "gray",
                "-fps_mode",
                "passthrough",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            reference.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(reference.stdout, expected, "{name}: decoded frame pixels");
        println!("{name}: four decoded frames match independent reference");
    }
}
