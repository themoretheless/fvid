use fvid::media::CopyOptions;
use std::path::PathBuf;
fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-remux-plan-capacity/edited.m4a")
}
#[test]
fn edited_aac_reserved_capacity_is_included_in_remux_plan_budget() {
    let options = CopyOptions {
        max_controlled_bytes: Some(20 << 10),
        ..Default::default()
    };
    let result = fvid_media::owned_mp4_remux::plan_remux(&source(), &options);
    assert!(
        matches!(result,Err(e) if e.contains("controlled memory budget exceeded")),
        "reserved capacity must count even when the edit emits only two packets"
    );
}
#[test]
fn generous_budget_accepts_edited_aac_remux_plan() {
    let options = CopyOptions {
        max_controlled_bytes: Some(2 << 20),
        ..Default::default()
    };
    assert!(fvid_media::owned_mp4_remux::plan_remux(&source(), &options).is_ok());
}
#[test]
fn budget_refusal_preserves_destination_and_generous_remux_decodes_exact_length() {
    let dir = std::env::temp_dir().join(format!("fvid-remux-plan-capacity-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let output = dir.join("out.mka");
    let tight = CopyOptions {
        max_controlled_bytes: Some(20 << 10),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_mp4_remux::remux(&source(), &output, &tight)
            .unwrap_err()
            .contains("controlled memory budget exceeded")
    );
    assert!(!output.exists());
    let ample = CopyOptions {
        max_controlled_bytes: Some(2 << 20),
        ..Default::default()
    };
    let result = fvid_media::owned_mp4_remux::remux(&source(), &output, &ample).unwrap();
    assert_eq!(result.packets, 2);
    let stats =
        fvid::media::decode_audio(&output, &dir.join("out.wav"), &CopyOptions::default()).unwrap();
    assert_eq!(stats.sample_rate, 48000);
    assert_eq!(stats.channels, 1);
    assert_eq!(stats.sample_frames, 2048);
}
