use std::{env, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-env-changed=FVID_FFMPEG_PREFIX");
    let prefix = env::var_os("FVID_FFMPEG_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
                PathBuf::from("/opt/homebrew")
            } else {
                PathBuf::from("/usr")
            }
        });
    let include = prefix.join("include");
    if !include.join("libavformat/avformat.h").exists() {
        panic!("Install FFmpeg development headers and set FVID_FFMPEG_PREFIX to their prefix");
    }
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    for library in ["avformat", "avcodec", "avutil", "avfilter"] {
        println!("cargo:rustc-link-lib={library}");
    }
    let bindings = bindgen::Builder::default()
        .header_contents("fvid_av.h", "#include <libavformat/avformat.h>\n#include <libavcodec/avcodec.h>\n#include <libavcodec/codec_desc.h>\n#include <libavutil/pixdesc.h>\n#include <libavfilter/avfilter.h>\n#include <libavfilter/buffersrc.h>\n#include <libavfilter/buffersink.h>\n")
        .clang_arg(format!("-I{}", include.display()))
        .allowlist_function("av.*")
        .allowlist_type("AV.*")
        .allowlist_var("AV.*|LIBAV.*")
        .derive_debug(false).layout_tests(false).generate_comments(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate().expect("FFmpeg headers must be readable by libclang");
    bindings
        .write_to_file(PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("av.rs"))
        .unwrap();
}
