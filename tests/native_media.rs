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

#[test]
fn interval_counts_presentation_frames_after_reference_preroll() {
    use std::time::Duration;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4");
    let range = Some((Duration::from_millis(40), Duration::from_millis(120)));
    let stats = fvid::native_media::decode_video_interval(&path, range).unwrap();
    assert_eq!(stats.video_frames, 2);
    let fractional = Some((
        Duration::from_nanos(33_333_333),
        Duration::from_nanos(33_333_334),
    ));
    assert_eq!(
        fvid::native_media::decode_video_interval(&path, fractional)
            .unwrap()
            .video_frames,
        1
    );
    assert!(
        fvid::native_media::decode_video_interval(
            &path,
            Some((Duration::from_secs(1), Duration::ZERO))
        )
        .is_err()
    );
}

#[cfg(not(feature = "videotoolbox"))]
#[test]
fn raw_main10_seek_matches_sequential_ten_bit_samples() {
    use fvid::playback_native::{NativeReader, RawFrame};
    use std::{io::Cursor, time::Duration};
    let mut reader = NativeReader::without_memory_limit(Cursor::new(include_bytes!(
        "fixtures/hevc/main10-ipb.mp4"
    )))
    .unwrap();
    let mut expected = Vec::new();
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        let RawFrame::Avc { picture, .. } = frame else {
            panic!("lost sample depth");
        };
        let mut bytes = Vec::new();
        picture.write_planar(&mut bytes).unwrap();
        expected.push((reader.frame_interval().unwrap(), bytes));
    }
    for index in [10, 0, 16, 3] {
        let (start, end, scale) = expected[index].0;
        let target =
            Duration::from_nanos(((start + end) * 1_000_000_000 / (2 * u128::from(scale))) as u64);
        let raw = reader.seek_raw(target).unwrap().unwrap();
        let RawFrame::Avc { picture, .. } = raw else {
            panic!("seek lost sample depth");
        };
        assert_eq!(picture.bit_depth, 10);
        let mut bytes = Vec::new();
        picture.write_planar(&mut bytes).unwrap();
        assert_eq!(reader.frame_interval(), Some(expected[index].0));
        assert!(
            bytes == expected[index].1,
            "sample mismatch at frame {index}"
        );
    }
}
