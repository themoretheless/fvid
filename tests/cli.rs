use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn publishes_success_and_preserves_existing_file() {
    let d = Directory::new();
    let input = d.0.join("in.y4m");
    let output = d.0.join("out.y4m");
    let bytes = b"YUV4MPEG2 W2 H2 F1:1 Ip C420jpeg\nFRAME\n\x01\x02\x03\x04\x05\x06";
    fs::write(&input, bytes).unwrap();
    let first = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(first.status.success());
    assert_eq!(fs::read(&output).unwrap(), bytes);
    let second = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .arg(&input)
        .arg(&output)
        .arg("--hflip")
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert_eq!(fs::read(&output).unwrap(), bytes);
    assert_eq!(fs::read_dir(&d.0).unwrap().count(), 2);
}
#[test]
fn corrupt_second_frame_never_publishes_partial_file() {
    let d = Directory::new();
    let input = d.0.join("in.y4m");
    let output = d.0.join("out.y4m");
    fs::write(
        &input,
        b"YUV4MPEG2 W2 H2 F1:1 Ip C420jpeg\nFRAME\n\x01\x02\x03\x04\x05\x06FRAME\n\x01",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output.exists());
    assert_eq!(fs::read_dir(&d.0).unwrap().count(), 1);
}
