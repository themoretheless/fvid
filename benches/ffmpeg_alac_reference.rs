//! Explicit ALAC MP4 and Matroska sample reference comparisons.
use fvid::native_export::export_audio_pcm_selected as export;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir() -> Dir {
    let p = std::env::temp_dir().join(format!(
        "fvid-alac-media-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/alac")
        .join(name)
}
fn reference_decoder_matches_all_alac_mp4_fixtures() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let d = dir();
    for name in [
        "mono-16.m4a",
        "mono-24.m4a",
        "stereo-16.m4a",
        "stereo-24.m4a",
        "stereo-pair-24.m4a",
        "silence-16.m4a",
        "noise-24.m4a",
    ] {
        let source = fixture(name);
        let output = d.0.join(format!("{name}.f32le"));
        export(&source, &output, None, 1.0, None, None, None, None, None).unwrap();
        let result = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&source)
            .args(["-f", "f32le", "pipe:1"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read(output).unwrap(), result.stdout, "{name}");
    }
}

fn matroska_alac_uses_owned_timeline() {
    let d = dir();
    let source = fixture("stereo-24.mka");
    assert!(fvid::native_media::is_owned_audio_source(&source).unwrap());
    let info = fvid::native_media::audio_source_info_selected(&source, None).unwrap();
    assert_eq!(info.codec, "alac");
    let full = d.0.join("full.f32le");
    let stats = export(&source, &full, None, 1.0, None, None, None, None, None).unwrap();
    let bytes = std::fs::read(&full).unwrap();
    if let Some(binary) = std::env::var_os("FVID_REFERENCE_FFMPEG") {
        let result = std::process::Command::new(binary)
            .args(["-v", "error", "-i"])
            .arg(&source)
            .args(["-f", "f32le", "-"])
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(bytes, result.stdout);
    }
    let window = d.0.join("window.f32le");
    let from = Duration::from_millis(10);
    let to = Duration::from_millis(50);
    export(
        &source,
        &window,
        Some((from, to)),
        1.0,
        None,
        None,
        Some(0),
        None,
        None,
    )
    .unwrap();
    let start = (from.as_nanos() * u128::from(stats.sample_rate)).div_ceil(1_000_000_000) as usize;
    let end = (to.as_nanos() * u128::from(stats.sample_rate)).div_ceil(1_000_000_000) as usize;
    let stride = usize::from(stats.channels) * 4;
    assert_eq!(
        std::fs::read(window).unwrap(),
        bytes[start * stride..end * stride]
    );
    #[cfg(feature = "media")]
    {
        let output = d.0.join("api.f32le");
        fvid::media::decode_audio(&source, &output, &Default::default()).unwrap();
        assert_eq!(std::fs::read(output).unwrap(), bytes);
    }
}


fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("Set FVID_REFERENCE_FFMPEG for this explicit reference benchmark");
    reference_decoder_matches_all_alac_mp4_fixtures();
    matroska_alac_uses_owned_timeline();
    println!("ALAC MP4 and Matroska reference comparisons passed");
}
