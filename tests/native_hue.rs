use fvid::{
    media_info::DecodeTransform, native_geometry::GeometryFrame, native_pixels::PixelFilters,
};
#[test]
fn native_filter_pipeline_uses_owned_hue_before_other_pixel_filters() {
    let request = DecodeTransform {
        hue: Some("h=90".into()),
        negate: Some("".into()),
        ..Default::default()
    };
    let filters = PixelFilters::from_request(&request).unwrap();
    assert!(!filters.is_empty());
    let mut frame = GeometryFrame {
        width: 2,
        height: 2,
        subsampling: Some([2, 2]),
        data: vec![16, 64, 128, 235, 160, 96],
    };
    filters.apply(&mut frame, 8).unwrap();
    assert_eq!(frame.data, [239, 191, 127, 20, 95, 95]);
    let invalid = DecodeTransform {
        hue: Some("h=t".into()),
        ..Default::default()
    };
    assert!(PixelFilters::from_request(&invalid).is_err());
}
#[test]
fn mp4_request_and_cli_decode_with_owned_hue_without_ffmpeg() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let request = DecodeTransform {
        hue: Some("h=90:s=1.2".into()),
        ..Default::default()
    };
    let stats = fvid::native_media::decode_video_request(&path, &request).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.video_frames, 25);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            path.to_str().unwrap(),
            "--hue",
            "h=90:s=1.2",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["backend"],
        "fvid"
    );
}
