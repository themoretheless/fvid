//! Optional search directory for the benchmark that links libavutil directly.
//! The library never links FFmpeg and needs no FFmpeg headers or bindgen.
fn main() {
    #[cfg(feature = "legacy-ffmpeg")]
    {
        println!("cargo:rerun-if-env-changed=FVID_FFMPEG_PREFIX");
        let prefix = std::env::var_os("FVID_FFMPEG_PREFIX")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
                Ok("macos") => "/opt/homebrew".into(),
                Ok("windows") => r"C:\ffmpeg-shared".into(),
                _ => "/usr".into(),
            });
        println!(
            "cargo:rustc-link-search=native={}",
            prefix.join("lib").display()
        );
    }
}
