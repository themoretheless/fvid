use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-{}-{}-{}",
            prefix,
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}

#[test]
fn lossless_y4m_eligible_and_write_succeed_under_single_open() {
    let dir = TempDir::new("lossless-y4m");
    let y4m = dir.path().join("sample.y4m");
    let mut bytes = b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420\n".to_vec();
    for val in [32u8, 96u8] {
        bytes.extend_from_slice(b"FRAME\n");
        bytes.extend_from_slice(&[val, val, val, val, 128, 128]);
    }
    std::fs::write(&y4m, &bytes).expect("write y4m");

    assert!(fvid::native_lossless_y4m::eligible(&y4m).expect("eligible y4m"));
    assert!(fvid::native_lossless::eligible(&y4m).expect("lossless eligible y4m"));

    let mkv = dir.path().join("out.mkv");
    let mut out = std::fs::File::create(&mkv).expect("create mkv");
    let (stats, event) = fvid::native_lossless_y4m::write(
        &y4m,
        &mut out,
        &Default::default(),
        &Default::default(),
        None,
        None,
    )
    .expect("write lossless mkv from y4m");
    assert_eq!(stats.video_frames, 2);
    assert_eq!(event.packets, 2);

    // WebM/Matroska lossless eligibility and write via single-scan open_lossless_source
    assert!(fvid::native_lossless_y4m::eligible(&mkv).expect("eligible mkv"));
    let mkv2 = dir.path().join("out2.mkv");
    let mut out2 = std::fs::File::create(&mkv2).expect("create mkv2");
    let (stats2, _) = fvid::native_lossless_y4m::write(
        &mkv,
        &mut out2,
        &Default::default(),
        &Default::default(),
        None,
        None,
    )
    .expect("write lossless mkv from mkv");
    assert_eq!(stats2.video_frames, 2);
}

#[test]
fn overlay_eligible_and_overlay_video_transformed_single_pass() {
    let dir = TempDir::new("overlay-single");
    let main = dir.path().join("main.y4m");
    let fg = dir.path().join("fg.y4m");
    let mut main_bytes = b"YUV4MPEG2 W4 H4 F2:1 Ip A1:1 C420\n".to_vec();
    let mut fg_bytes = b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420\n".to_vec();
    for val in [16u8, 64u8] {
        main_bytes.extend_from_slice(b"FRAME\n");
        main_bytes.extend_from_slice(&[val; 16]);
        main_bytes.extend_from_slice(&[128; 8]);
        fg_bytes.extend_from_slice(b"FRAME\n");
        fg_bytes.extend_from_slice(&[200, 200, 200, 200, 128, 128]);
    }
    std::fs::write(&main, &main_bytes).expect("write main");
    std::fs::write(&fg, &fg_bytes).expect("write fg");

    assert!(fvid::native_export::overlay_eligible(&main).expect("overlay eligible y4m"));
    let non_eligible = fixture("wave-ogg-flac-plan/wave-24in32-valid.wav");
    assert!(!fvid::native_export::overlay_eligible(&non_eligible).expect("non-eligible wav"));

    let dest = dir.path().join("overlay.mkv");
    let stats = fvid::native_export::overlay_video(&main, &fg, &dest, 0, 0, None, None)
        .expect("overlay_video");
    assert_eq!(stats.video_frames, 2);
    assert!(dest.exists());
    assert!(fvid::native_export::overlay_eligible(&dest).expect("overlay eligible mkv"));
}
