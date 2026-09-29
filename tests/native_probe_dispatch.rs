use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}
#[cfg(feature = "media")]
#[test]
fn legacy_api_reexports_native_description_types_and_preserves_results() {
    for name in ["video.mp4", "audio/aac-mono-44k.aac", "vp9/adaptive.webm"] {
        let source = fixture(name);
        let native: fvid::media::MediaInfo = fvid::native_probe::probe(&source).unwrap();
        let api = fvid::media::probe(&source).unwrap();
        assert_eq!(serde_json::to_value(native).unwrap(), serde_json::to_value(api).unwrap());
    }
}
#[test]
fn unknown_formats_are_distinct_from_corrupt_known_containers() {
    let path = std::env::temp_dir().join(format!("fvid-probe-dispatch-{}",std::process::id()));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }
    let _cleanup = Cleanup(path.clone());
    std::fs::write(&path, b"not a media container").unwrap();
    assert!(fvid::native_probe::try_probe_as(&path, None).unwrap().is_none());
    assert!(fvid::native_probe::probe(&path).unwrap_err().contains("does not yet support"));
    std::fs::write(&path, [0x1a,0x45,0xdf,0xa3]).unwrap();
    assert!(fvid::native_probe::try_probe_as(&path, None).is_err());
    std::fs::write(&path, [0,0,0,24,b'f',b't',b'y',b'p']).unwrap();
    assert!(fvid::native_probe::try_probe_as(&path, None).is_err());
    // A hint is authoritative; bytes from a different supported container must
    // not silently select another parser.
    assert!(fvid::native_probe::probe_as(&fixture("video.mp4"), Some("aac")).is_err());
}
