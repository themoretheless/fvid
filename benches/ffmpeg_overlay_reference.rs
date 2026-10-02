//! Explicit overlay export decoding reference benchmark.
use fvid::{native_export, native_geometry::VideoGeometry, playback_native::NativeReader};
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-overlay-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn samples(path: &Path) -> Vec<Vec<u8>> {
    let mut reader =
        NativeReader::software(BufReader::new(File::open(path).unwrap()), usize::MAX).unwrap();
    let mut result = vec![];
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        let [w, h] = reader.dimensions();
        result.push(
            VideoGeometry::default()
                .apply_cropped_display(&frame, w, h, reader.rotation(), reader.insets())
                .unwrap()
                .data,
        );
    }
    result
}
fn main() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").expect("set reference executable");
    let d = dir("benchmark");
    for (i, (name, pixel)) in [
        ("video.mp4", "yuv420p"),
        ("hevc/main-ipb.mp4", "yuv420p"),
        ("hevc/main10-ipb.mp4", "yuv420p10le"),
        ("audio/two-audio.mp4", "yuv420p"),
        ("vp9/adaptive.webm", "yuv420p"),
        ("vp9/odd10.webm", "yuv420p10le"),
        ("vp9/lossless12.webm", "yuv420p12le"),
        ("av1/ramp.webm", "yuv420p"),
        ("av1/tiles.webm", "yuv420p"),
        ("display/vp9-rot90.mkv", "yuv420p"),
        ("geometry/422.y4m", "yuv422p"),
    ]
    .iter()
    .enumerate()
    {
        let source = fixture(name);
        let output = d.0.join(format!("{i}.mkv"));
        let expected = samples(&source);
        native_export::overlay_video(&source, &source, &output, 0, 0, None, None).unwrap();
        let result = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-f", "rawvideo", "-pix_fmt", pixel, "-"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, expected.concat(), "{name}");
    }
    println!("11 overlay export reference comparisons passed");
}
