//! Large sparse setups stay compatible with configured export admission.
use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors")
        .join(name)
}
#[test]
fn large_vorbis_setups_export_natively_without_bypassing_memory_policy() {
    use std::io::{Read, Seek, SeekFrom};
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("fvid-vorbis-large-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    for variant in ["entries", "multiplicands"] {
        let source = fixture(&format!("vorbis-large-{variant}.ogg"));
        let output = directory.0.join(format!("{variant}.wav"));
        let stats = fvid_media::decode_audio(
            &source,
            &output,
            &fvid_control::CopyOptions {
                max_controlled_bytes: Some(64 * 1024 * 1024),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 411);
        assert_eq!(stats.decoded_frames, 6);
        let mut file = std::fs::File::open(output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.data_bytes),
            (8000, 2, 411 * 8)
        );
        assert!(info.float);
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut pcm = vec![0; info.data_bytes as usize];
        file.read_exact(&mut pcm).unwrap();
        assert!(pcm.iter().all(|&b| b == 0));
        let output = directory.0.join(format!("{variant}-refused.wav"));
        let error = fvid_media::decode_audio(
            &source,
            &output,
            &fvid_control::CopyOptions {
                max_controlled_bytes: Some(512 * 1024),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            error.contains("controlled memory budget exceeded"),
            "{error}"
        );
        assert!(!output.exists());
    }
}
