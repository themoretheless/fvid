use std::path::Path;

#[test]
fn decodes_owned_avc_and_hevc_without_media_dependency() {
    for (file, format) in [
        ("video.mp4", "yuv420p"),
        ("hevc/main-ipb.mp4", "yuv420p"),
        ("hevc/main10-ipb.mp4", "yuv420p10le"),
    ] {
        let stats = fvid::native_media::decode_video(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(file),
        )
        .unwrap();
        assert!(stats.video_frames > 1, "{file}: {stats:?}");
        assert!(stats.width > 0 && stats.height > 0);
        if file.starts_with("hevc/") {
            assert_eq!(stats.video_frames, 17);
        }
        assert_eq!(stats.decode_errors, 0);
        #[cfg(not(feature = "videotoolbox"))]
        {
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.pixel_format, format);
        }
        let _ = format;
    }
}

#[test]
fn invalid_input_is_not_silently_retried() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    assert!(fvid::native_media::decode_video(&path).is_err());
}

#[cfg(feature = "media")]
#[test]
fn public_media_api_uses_native_decode() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let stats = fvid::media::decode_video(&path).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert!(stats.video_frames > 0);
}

#[cfg(not(feature = "videotoolbox"))]
#[test]
fn raw_main10_preserves_every_decoded_sample() {
    use fvid::playback_native::{NativeReader, RawFrame};
    let mut reader = NativeReader::without_memory_limit(std::io::Cursor::new(include_bytes!(
        "fixtures/hevc/main10-ipb.mp4"
    )))
    .unwrap();
    let mut output = Vec::new();
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        let RawFrame::Avc { picture, .. } = frame else {
            panic!("10-bit planes were reduced");
        };
        assert_eq!(picture.bit_depth, 10);
        picture.write_planar(&mut output).unwrap();
    }
    assert_eq!(
        output.as_slice(),
        include_bytes!("fixtures/hevc/main10-ipb.yuv")
    );
}
