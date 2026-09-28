use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn directory() -> Directory {
    let path = std::env::temp_dir().join(format!(
        "fvid-export-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    Directory(path)
}
#[test]
fn exports_main10_exactly_and_never_overwrites() {
    let dir = directory();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4");
    let output = dir.0.join("out.y4m");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "export-y4m"])
        .arg(&source)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("\"video_frames\":17"));
    let bytes = std::fs::read(&output).unwrap();
    let end = bytes.iter().position(|b| *b == b'\n').unwrap() + 1;
    let header = std::str::from_utf8(&bytes[..end]).unwrap();
    assert!(header.contains("F30:1"), "{header}");
    assert!(header.contains("C420p10"), "{header}");
    let oracle = include_bytes!("fixtures/hevc/main10-ipb.yuv");
    let frame_size = oracle.len() / 17;
    let mut payload = &bytes[end..];
    for expected in oracle.chunks_exact(frame_size) {
        assert!(payload.starts_with(b"FRAME\n"));
        payload = &payload[6..];
        assert!(payload[..frame_size] == *expected);
        payload = &payload[frame_size..];
    }
    assert!(payload.is_empty());
    assert!(fvid::native_export::export_y4m(&source, &output).is_err());
    assert!(std::fs::read(&output).unwrap() == bytes);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}
#[test]
fn decode_failure_leaves_no_output_or_temporary_file() {
    let dir = directory();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    assert!(fvid::native_export::export_y4m(&source, &dir.0.join("bad.y4m")).is_err());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[test]
fn truncated_input_cleans_partial_export() {
    let dir = directory();
    let source = dir.0.join("truncated.y4m");
    let mut input = b"YUV4MPEG2 W2 H2 F30:1 Ip A1:1 C420\nFRAME\n".to_vec();
    input.extend_from_slice(&[16, 16, 16, 16, 128, 128]);
    input.extend_from_slice(b"FRAME\n");
    std::fs::write(&source, input).unwrap();
    let output = dir.0.join("out.y4m");
    assert!(fvid::native_export::export_y4m(&source, &output).is_err());
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}
