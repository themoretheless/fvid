//! Explicit reference benchmark; ordinary tests and fixture generators use no FFmpeg.
use std::{path::Path, process::Command, time::Instant};
fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    let ffmpeg = std::env::var_os("FVID_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for name in [
        "ima4-ramp-edits.mov",
        "ima4-adaptive-stereo-edits.mov",
        "ima-wav-stereo-edits.mov",
    ] {
        let source = root.join(name);
        let mut reader = fvid_media::owned_mp4::Mp4Reader::open(
            std::fs::File::open(&source).unwrap(),
            Default::default(),
        )
        .unwrap();
        let track = reader.tracks()[0].clone();
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        let decode = || {
            if track.codec == *b"ima4" {
                fvid_media::owned_ima4::Ima4Decoder::new(track.sample_rate, track.channels)
                    .unwrap()
                    .decode_pcm(&packet)
                    .unwrap()
            } else {
                fvid_media::owned_ima_wav::ImaWavDecoder::new(
                    &track.configuration,
                    track.sample_rate,
                    track.channels,
                )
                .unwrap()
                .decode_pcm(&packet)
                .unwrap()
            }
        };
        let expected: Vec<u8> = decode().iter().flat_map(|s| s.to_le_bytes()).collect();
        let reference = Command::new(&ffmpeg)
            .args(["-v", "error", "-ignore_editlist", "1", "-i"])
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
            reference.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(
            reference.stdout, expected,
            "{name}: independent reference PCM"
        );
        let start = Instant::now();
        for _ in 0..10000 {
            std::hint::black_box(decode());
        }
        let frames = expected.len() / 4 / usize::from(track.channels);
        println!(
            "{name}: {frames} frames match reference; owned {:.1} ns/frame (includes packet allocation)",
            start.elapsed().as_nanos() as f64 / 10000.0 / frames as f64
        );
    }
}
