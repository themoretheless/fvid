use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=FVID_FFMPEG_PREFIX");
    println!("cargo:rerun-if-env-changed=LIBCLANG_PATH");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let prefix = env::var_os("FVID_FFMPEG_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if target_os == "macos" {
                PathBuf::from("/opt/homebrew")
            } else if target_os == "windows" {
                PathBuf::from(r"C:\ffmpeg-shared")
            } else {
                PathBuf::from("/usr")
            }
        });
    let include = prefix.join("include");
    if !include.join("libavformat/avformat.h").exists() {
        panic!(
            "FFmpeg development headers not found under {}. Install a shared+dev FFmpeg build and set FVID_FFMPEG_PREFIX (on Windows run scripts/setup_ffmpeg_windows.ps1).",
            prefix.display()
        );
    }
    let lib = prefix.join("lib");
    if !lib.is_dir() {
        panic!(
            "FFmpeg lib directory missing at {}. Shared Windows builds need include/, lib/ (*.lib), and bin/ (*.dll).",
            lib.display()
        );
    }
    println!("cargo:rustc-link-search=native={}", lib.display());
    for library in ["avformat", "avcodec", "avutil", "avfilter"] {
        // MSVC import libs are avformat.lib; MinGW uses libavformat.dll.a — rustc -l avformat
        // resolves both when the search path is correct.
        println!("cargo:rustc-link-lib={library}");
    }
    let bin = prefix.join("bin");
    if target_os == "windows" && bin.is_dir() {
        // Help runtime discovery when running tests from the build tree.
        println!("cargo:rustc-env=FVID_FFMPEG_BIN={}", bin.display());
        println!("cargo:warning=Add {} to PATH so FFmpeg DLLs load at runtime", bin.display());
    }
    let builder = bindgen::Builder::default()
        .header_contents(
            "fvid_av.h",
            "#include <libavformat/avformat.h>\n#include <libavcodec/avcodec.h>\n#include <libavcodec/codec_desc.h>\n#include <libavutil/pixdesc.h>\n#include <libavutil/hwcontext.h>\n#include <libavfilter/avfilter.h>\n#include <libavfilter/buffersrc.h>\n#include <libavfilter/buffersink.h>\n",
        )
        .clang_arg(format!("-I{}", include.display()))
        .allowlist_function("av.*")
        .allowlist_type("AV.*")
        .allowlist_var("AV.*|LIBAV.*")
        .derive_debug(false)
        .layout_tests(false)
        .generate_comments(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
    let bindings = builder
        .generate()
        .expect("FFmpeg headers must be readable by libclang; set LIBCLANG_PATH on Windows to LLVM bin");
    bindings
        .write_to_file(PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("av.rs"))
        .unwrap();
}
