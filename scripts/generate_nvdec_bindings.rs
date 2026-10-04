//! Developer-only generator; bindgen 0.72.1 and libclang are not normal build dependencies.
//! Usage: generator pinned-cuviddec.h output.rs
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3);
    let header = std::fs::read_to_string(&args[1]).unwrap();
    assert!(header.contains("unsigned int CodecReserved[1024]"));
    let license = &header[..header.find("*/").unwrap() + 2];
    // Header parsing only needs the CUDA opaque/pointer aliases for unrelated
    // declarations. No CUDA SDK code or linked declarations are emitted.
    let prelude = "#define __cuda_cuda_h__\n#define CUDAAPI\n#define CUDA_VERSION 9000\ntypedef int CUresult;\ntypedef void *CUcontext;\ntypedef void *CUstream;\ntypedef unsigned long long CUdeviceptr;\n";
    let wrapper = format!("{prelude}\n{header}");
    let bindings = bindgen::Builder::default()
        .header_contents("cuviddec-wrapper.h", &wrapper)
        .allowlist_type("CUVIDPICPARAMS")
        .allowlist_var("^$")
        .generate_comments(false)
        .layout_tests(false)
        .derive_default(true)
        .generate()
        .unwrap();
    let prefix = "\n// Generated with bindgen 0.72.1 from NVIDIA/video-sdk-samples\n// aa3544dcea2fe63122e4feb83bf805ea40e58dbe, cuviddec.h (SDK 8.1).\n// Data ABI only; no linked symbols or build-time SDK/FFmpeg dependency.\n#![allow(dead_code, non_camel_case_types, non_snake_case, non_upper_case_globals,\n    unnecessary_transmutes, unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]\n";
    std::fs::write(&args[2], format!("{license}{prefix}{bindings}")).unwrap();
}
