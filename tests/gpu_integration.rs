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

#[test]
fn shader_request_rejects_cpu_and_invalid_source_without_publication() {
    let directory = Directory::new();
    let input = directory.0.join("input.y4m");
    let output = directory.0.join("output.y4m");
    let shader = directory.0.join("effect.wgsl");
    fs::write(
        &input,
        b"YUV4MPEG2 W2 H2 F1:1 Ip C420jpeg\nFRAME\n\x01\x02\x03\x04\x05\x06",
    )
    .unwrap();
    for source in ["not wgsl", include_str!("../shaders/negate.wgsl")] {
        fs::write(&shader, source).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
            .arg(&input)
            .arg(&output)
            .args(["--backend", "cpu", "--shader"])
            .arg(&shader)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!output.exists());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    }
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires a physical GPU; set FVID_SHADER_BACKEND"]
fn cli_shader_chain_executes_without_intermediate_transfers() {
    let backend =
        std::env::var("FVID_SHADER_BACKEND").expect("explicit physical GPU backend required");
    assert!(["metal", "vulkan", "dx12", "gl"].contains(&backend.as_str()));
    let directory = Directory::new();
    let input = directory.0.join("input.y4m");
    let output = directory.0.join("output.y4m");
    fs::write(
        &input,
        b"YUV4MPEG2 W2 H2 F1:1 Ip C420jpeg\nFRAME\n\x0a\x1e\x50\xb4\x80\x80",
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .arg(&input)
        .arg(&output)
        .args(["--backend", &backend, "--hflip", "--shader"])
        .arg(root.join("shaders/negate.wgsl"))
        .args(["--then", "--shader"])
        .arg(root.join("shaders/boxblur.wgsl"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let actual = fs::read(output).unwrap();
    let negated = [225u32, 245, 75, 175];
    let mut expected = vec![];
    for y in 0isize..2 {
        for x in 0isize..2 {
            let mut sum = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    sum += negated[((y + dy).clamp(0, 1) * 2 + (x + dx).clamp(0, 1)) as usize];
                }
            }
            expected.push((sum / 9) as u8);
        }
    }
    expected.extend([128, 128]);
    assert_eq!(&actual[actual.len() - 6..], expected);
    let diagnostics = String::from_utf8_lossy(&result.stderr);
    assert!(diagnostics.contains("uploads=1 downloads=1"));
    assert!(diagnostics.contains("filter_passes=2"));
    assert!(diagnostics.contains(&format!("backend={backend}")));
}
