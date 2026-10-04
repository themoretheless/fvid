//! Explicit float-domain reference qualification, excluded from ordinary tests.
use std::{path::Path, process::Command};
fn main() {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    let input = std::fs::read(fixtures.join("grayworld-grid.rgba_f32")).unwrap();
    let mut planar = Vec::new();
    let mut actual = Vec::new();
    for frame in input.chunks_exact(8 * 8 * 4 * 4) {
        let mut rgb: Vec<f32> = frame
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        for channel in [1, 2, 0, 3] {
            for pixel in rgb.chunks_exact(4) {
                planar.extend(pixel[channel].to_le_bytes());
            }
        }
        fvid_media::owned_grayworld::GrayWorld
            .apply_rgb_f32(&mut rgb, 8, 8, 4)
            .unwrap();
        actual.extend(rgb);
    }
    let source = std::env::temp_dir().join(format!(
        "fvid-grayworld-reference-{}.raw",
        std::process::id()
    ));
    std::fs::write(&source, &planar).unwrap();
    let reference = Command::new(std::env::var("FVID_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()))
        .args([
            "-nostdin",
            "-v",
            "error",
            "-filter_threads",
            "1",
            "-f",
            "rawvideo",
            "-pixel_format",
            "gbrapf32le",
            "-video_size",
            "8x8",
            "-i",
        ])
        .arg(&source)
        .args([
            "-vf",
            "grayworld",
            "-frames:v",
            "3",
            "-pix_fmt",
            "gbrapf32le",
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
    let mut expected = Vec::new();
    for frame in reference.stdout.chunks_exact(8 * 8 * 4 * 4) {
        for i in 0..64 {
            for plane in [2, 0, 1, 3] {
                let at = (plane * 64 + i) * 4;
                expected.push(f32::from_le_bytes(frame[at..at + 4].try_into().unwrap()));
            }
        }
    }
    assert_eq!(actual.len(), expected.len());
    let max = actual
        .iter()
        .zip(&expected)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let normalized = actual
        .iter()
        .zip(&expected)
        .map(|(a, b)| (a - b).abs() / b.abs().max(1.0) / f32::EPSILON)
        .fold(0.0f32, f32::max);
    println!(
        "components={} max_absolute={max} max_scaled_epsilon={normalized}",
        actual.len()
    );
    assert!(normalized <= 8.0, "float compatibility bound exceeded");
    if std::env::var_os("FVID_WRITE_SYNTHETIC_REFERENCES").is_some() {
        std::fs::write(
            fixtures.join("grayworld-reference.rgba_f32"),
            expected
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    }
    std::fs::remove_file(source).unwrap();
}
