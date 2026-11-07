use fvid::media::{self, CopyOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fvid-probe-single-open-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn probe_flac_mpeg_y4m_and_signature_single_open() {
    let flac = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/flac-4ch-quad.flac");
    let mp3 = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let options = CopyOptions::default();

    let flac_info = media::probe_with_options(flac, &options).expect("FLAC probe should succeed");
    assert_eq!(flac_info.format, "flac");
    assert_eq!(flac_info.streams[0].codec, "flac");

    let mp3_info = media::probe_with_options(mp3, &options).expect("MPEG probe should succeed");
    assert_eq!(mp3_info.format, "mp3");
    assert_eq!(mp3_info.streams[0].codec, "mp3");

    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let flac_err = media::probe_as_with_options(wav, Some("flac"), &options)
        .expect_err("non-FLAC input with format=flac should fail");
    assert!(flac_err.contains("bare FLAC audio signature"));

    let mpeg_err = media::probe_as_with_options(wav, Some("mp3"), &options)
        .expect_err("non-MPEG input with format=mp3 should fail");
    assert!(mpeg_err.contains("bare MPEG audio signature"));

    let dir = TempDir::new();
    let y4m_path = dir.0.join("sample.y4m");
    let mut y4m_bytes = b"YUV4MPEG2 W2 H2 F25:1 Ip A1:1 C420\nFRAME\n".to_vec();
    y4m_bytes.extend_from_slice(&[128u8; 6]);
    std::fs::write(&y4m_path, &y4m_bytes).unwrap();

    let y4m_info = media::probe_with_options(&y4m_path, &options)
        .expect("Y4M probe should succeed in a single header line pass");
    assert_eq!(y4m_info.format, "yuv4mpegpipe");
    assert_eq!(y4m_info.streams[0].duration, Some(1));
}
