use std::path::PathBuf;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-batch630-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn lossless_mp4_plan_overlay_and_pcm_export_reuse_opened_readers() {
    let dir = TempDir::new("reuse");
    let mp4_avc = PathBuf::from("tests/fixtures/playback-errors/avc-baseline-2x2.mp4");
    if mp4_avc.exists() {
        let transform = fvid::media::LosslessTransform::default();
        let plan = fvid::native_plan::transcode_lossless(&mp4_avc, &transform).unwrap();
        assert_eq!(plan.command, "transcode-lossless");
        assert!(!plan.streams.is_empty());

        let out_mkv = dir.0.join("out-avc-ffv1.mkv");
        let stats = fvid::native_export::transcode_mp4_ffv1(&mp4_avc, &out_mkv, None, None).unwrap();
        assert!(stats.video_frames >= 1);

        let overlay_plan = fvid::native_plan::overlay(&mp4_avc, &mp4_avc, 0, 0).unwrap();
        assert_eq!(overlay_plan.command, "overlay");
        assert_eq!(overlay_plan.streams[0].disposition, "decode_overlay");

        let out_overlay = dir.0.join("out-overlay.mkv");
        let ov_stats =
            fvid::native_export::overlay_video(&mp4_avc, &mp4_avc, &out_overlay, 0, 0, None, None)
                .unwrap();
        assert!(ov_stats.video_frames >= 1);
    }

    let mp4_aac = PathBuf::from("tests/fixtures/playback-errors/aac-lc-48k-stereo.mp4");
    if mp4_aac.exists() {
        let out_mka = dir.0.join("out-aac.mka");
        let stats = fvid::native_export::export_audio_pcm_selected(
            &mp4_aac,
            &out_mka,
            None,
            1.0,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(stats.sample_frames > 0);
    }
}
