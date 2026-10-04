//! Explicit developer tool; requires bindgen 0.72.1, not used by normal builds.
//! Arguments: pinned SDK 8.1 nvEncodeAPI.h, output Rust file.
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: generator nvEncodeAPI.h output.rs");
    let header = std::fs::read_to_string(&args[1]).unwrap();
    assert!(header.contains("#define NVENCAPI_MAJOR_VERSION 8\n#define NVENCAPI_MINOR_VERSION 1"));
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
    let prefix = "\n// Generated with bindgen 0.72.1 from NVIDIA/video-sdk-samples commit\n// aa3544dcea2fe63122e4feb83bf805ea40e58dbe, nvEncodeAPI.h (SDK 8.1).\n// Data/callback ABI only; no linked symbols, build-time SDK or FFmpeg dependency.\n#![allow(dead_code, non_camel_case_types, non_snake_case, non_upper_case_globals,\n    unused_imports, unnecessary_transmutes, unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]\n";
    std::fs::write(&args[2], format!("{license}{prefix}{bindings}")).unwrap();
}
