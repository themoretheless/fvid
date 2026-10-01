//! Emit checked-in PTX for `transform.cu` / `nv12.cu` via NVRTC (no MSVC/nvcc host compiler).
//!
//! ```sh
//! # Windows: put CUDA bin\x64 on PATH (nvrtc64_130_0.dll)
//! cargo run -p fvid-cuda --example emit_ptx --release
//! ```
#[cfg(any(target_os = "linux", target_os = "windows"))]
use cudarc::nvrtc::{CompileOptions, compile_ptx_with_opts};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::fs;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::path::PathBuf;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn main() {
    eprintln!("PTX generation requires Linux or Windows with NVIDIA NVRTC installed");
    std::process::exit(1);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("ptx");
    fs::create_dir_all(&out).expect("mkdir ptx");

    let arches = ["75", "80", "86", "89", "90", "100", "120"];
    let kernels = [
        ("transform", include_str!("../src/transform.cu")),
        ("nv12", include_str!("../src/nv12.cu")),
    ];

    for (name, src) in kernels {
        for arch in arches {
            let ptx = compile_ptx_with_opts(
                src,
                CompileOptions {
                    options: vec![format!("--gpu-architecture=compute_{arch}")],
                    name: Some(format!("fvid_{name}.cu")),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("NVRTC {name} compute_{arch}: {e:?}"));
            let path = out.join(format!("{name}_sm{arch}.ptx"));
            fs::write(&path, ptx.to_src().as_bytes()).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            println!(
                "wrote {} ({} bytes)",
                path.display(),
                fs::metadata(&path).unwrap().len()
            );
        }
    }
}
