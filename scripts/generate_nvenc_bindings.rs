//! Explicit developer tool; requires bindgen 0.72.1, not used by normal builds.
//! Arguments: pinned SDK 12.0 nvEncodeAPI.h, output Rust file.
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: generator nvEncodeAPI.h output.rs");
    let header = std::fs::read_to_string(&args[1]).unwrap().replace("\r\n", "\n");
    assert!(header.contains("#define NVENCAPI_MAJOR_VERSION 12\n#define NVENCAPI_MINOR_VERSION 0"));
    let license = &header[..header.find("*/").unwrap() + 2];
    let bindings = bindgen::Builder::default()
        .header(&args[1])
        .allowlist_type("NV_.*")
        .allowlist_type("PNV.*")
        .allowlist_type("GUID")
        .allowlist_var("^$")
        .generate_comments(false)
        .layout_tests(false)
        .derive_default(true)
        .override_abi(bindgen::Abi::System, ".*")
        .generate().unwrap();
    let prefix = "\n// Generated with bindgen 0.72.1 from NVIDIA/VideoProcessingFramework commit\n// 529fb192f22ec305b8b237ed9e6014e171339c81, nvEncodeAPI.h (SDK 12.0).\n// Data/callback ABI only; no linked symbols, build-time SDK or FFmpeg dependency.\n#![allow(dead_code, non_camel_case_types, non_snake_case, non_upper_case_globals,\n    unused_imports, unnecessary_transmutes, unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]\n";
    std::fs::write(&args[2], format!("{license}{prefix}{bindings}")).unwrap();
}
