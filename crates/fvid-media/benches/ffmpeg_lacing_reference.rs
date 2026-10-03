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
    let ffprobe = std::env::var_os("FVID_FFPROBE").unwrap_or_else(|| "ffprobe".into());
    for name in [
        "opus-lace-xiph.mka",
        "opus-lace-ebml.mka",
        "opus-lace-xiph-group.mka",
        "opus-lace-ebml-group.mka",
    ] {
        let source = root.join(name);
        let reference = Command::new(&ffprobe)
            .args([
                "-v",
                "error",
                "-show_packets",
                "-show_entries",
                "packet=pts,duration",
                "-of",
                "json",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            reference.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&reference.stdout).unwrap();
        let packets = json["packets"].as_array().unwrap();
        assert_eq!(packets.len(), 4);
        let clocks = if name.contains("group") {
            [(0, 22), (22, 23), (45, 22), (67, 23)]
        } else {
            [(0, 20), (20, 40), (60, 10), (70, 20)]
        };
        for (packet, (pts, duration)) in packets.iter().zip(clocks) {
            assert_eq!(packet["pts"].as_i64(), Some(pts), "{name}: packet PTS");
            assert_eq!(
                packet["duration"].as_i64(),
                Some(duration),
                "{name}: packet duration"
            );
        }
        let audio = Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&source)
            .args([
                "-map",
                "0:a:0",
                "-c:a",
                "pcm_f32le",
                "-f",
                "f32le",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            audio.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&audio.stderr)
        );
        assert_eq!(audio.stdout.len(), 4320 * 4, "{name}: decoded sample count");
        println!("{name}: container/codec packet clocks match reference; 4320 samples decode");
    }
}
