use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-backend-test-{}-{}",
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

fn rejects_without_publication(flags: &[&str]) {
    let directory = Directory::new();
    let input = directory.0.join("input.y4m");
    let output = directory.0.join("output.y4m");
    fs::write(
        &input,
        b"YUV4MPEG2 W2 H2 F1:1 Ip C420jpeg\nFRAME\n\x01\x02\x03\x04\x05\x06",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .arg(&input)
        .arg(&output)
        .args(flags)
        .output()
        .unwrap();
    assert!(
        !result.status.success(),
        "request unexpectedly succeeded: {flags:?}; {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!output.exists());
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn rejects_unknown_backend_without_publishing_output() {
    rejects_without_publication(&["--backend", "unknown-backend"]);
}

#[test]
fn rejects_invalid_device_index_without_publishing_output() {
    rejects_without_publication(&["--backend", "cpu", "--device", "not-an-index"]);
}

#[test]
fn explicit_gpu_device_failure_never_falls_back_to_cpu() {
    for backend in ["metal", "vulkan", "dx12", "gl", "cuda"] {
        rejects_without_publication(&["--backend", backend, "--device", "4294967295"]);
    }
}

#[test]
fn explicit_cpu_path_preserves_payload_and_reports_backend() {
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["-", "-", "--backend", "cpu"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"YUV4MPEG2 W1 H1 F1:1 Ip C444\nFRAME\n\x10\x80\xff")?;
            child.wait_with_output()
        })
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.stdout,
        b"YUV4MPEG2 W1 H1 F1:1 Ip C444\nFRAME\n\x10\x80\xff"
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("backend=cpu"));
}
