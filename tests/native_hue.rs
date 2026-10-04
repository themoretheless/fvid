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
        hue: Some("h=unknown".into()),
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

#[test]
fn mp4_lossless_export_applies_hue_and_preserves_every_frame() {
    use fvid::{native_geometry::VideoGeometry, playback_native::NativeReader};
    use std::{fs::File, io::BufReader};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let output =
        std::env::temp_dir().join(format!("fvid-native-hue-export-{}.mkv", std::process::id()));
    let request = fvid::media_info::LosslessTransform {
        hue: Some("h=90:s=0.5".into()),
        ..Default::default()
    };
    assert!(fvid::native_lossless::supports(&request));
    assert!(!fvid::native_lossless::identity(&request));
    let (geometry, filters) = fvid::native_lossless::configuration(&request).unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "transcode-lossless"])
        .arg(&source)
        .arg(&output)
        .args(["--hue", "h=90:s=0.5"])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&status.stdout).unwrap()["backend"],
        "fvid"
    );
    let mut original =
        NativeReader::software(BufReader::new(File::open(&source).unwrap()), usize::MAX).unwrap();
    let mut exported =
        NativeReader::software(BufReader::new(File::open(&output).unwrap()), usize::MAX).unwrap();
    let mut frames = 0;
    while let Some(frame) = original.read_frame_raw().unwrap() {
        let [w, h] = original.dimensions();
        let mut expected = geometry
            .apply_display(&frame, w, h, original.rotation())
            .unwrap();
        filters.apply(&mut expected, 8).unwrap();
        let decoded = exported.read_frame_raw().unwrap().unwrap();
        let [w, h] = exported.dimensions();
        let actual = VideoGeometry::default()
            .apply_display(&decoded, w, h, exported.rotation())
            .unwrap();
        assert_eq!(actual.data, expected.data, "frame {frames}");
        frames += 1;
    }
    assert_eq!(frames, 25);
    assert!(exported.read_frame_raw().unwrap().is_none());
    drop(exported);
    std::fs::remove_file(output).unwrap();
    // Timeline support is qualified by native_hue_timeline; retain admission here.
    assert!(fvid::native_lossless::supports(
        &fvid::media_info::LosslessTransform {
            hue: Some("h=t".into()),
            ..Default::default()
        }
    ));
}

#[test]
fn overlay_cli_keeps_constant_hue_on_owned_decode_and_export() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hue-8.y4m");
    let output = std::env::temp_dir().join(format!("fvid-overlay-hue-{}.mkv", std::process::id()));
    for operation in ["decode", "transcode-lossless"] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
        command.args(["media", operation]).arg(&source);
        if operation == "transcode-lossless" {
            command.arg(&output);
        }
        let result = command
            .arg("--overlay")
            .arg(&source)
            .args(["--hue", "h=90"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{operation}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(stats["backend"], "fvid");
        assert_eq!(stats["video_frames"], 1);
    }
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        std::io::Cursor::new(std::fs::read(&output).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.packets.len(), 1);
    let frame = fvid_media::owned_ffv1_decoder::Decoder::new(2, 2, 1 << 20)
        .unwrap()
        .decode(&reader.read_packet(0).unwrap())
        .unwrap();
    assert_eq!(frame.frame.data, [16, 64, 128, 235, 160, 160]);
    std::fs::remove_file(output).unwrap();
}
