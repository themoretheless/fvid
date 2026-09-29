use fvid::{
    native_geometry::VideoGeometry,
    playback_native::{NativeReader, RawFrame},
};
use std::{io::Cursor, path::Path};

#[test]
fn sample_plane_crop_flip_and_resize_have_exact_pixels() {
    // 4x4 luma with distinct values; two distinct 2x2 chroma planes.
    let mut data: Vec<u8> = (0..16).collect();
    data.extend([40, 41, 42, 43, 80, 81, 82, 83]);
    let frame = RawFrame::Yuv {
        data,
        width: 4,
        height: 4,
        luma_len: 16,
        chroma_len: 4,
        sx: 2,
        sy: 2,
    };
    let out = VideoGeometry {
        crop: Some([2, 0, 2, 4]),
        horizontal_flip: true,
        vertical_flip: true,
        scale: Some([4, 2]),
    }
    .apply(&frame, 4, 4)
    .unwrap();
    assert_eq!((out.width, out.height), (4, 2));
    assert_eq!(out.data, [11, 11, 10, 10, 3, 3, 2, 2, 41, 41, 81, 81]);
    for crop in [
        [1, 0, 2, 2],
        [0, 1, 2, 2],
        [0, 0, 0, 2],
        [2, 0, 4, 2],
        [usize::MAX, 0, 2, 2],
    ] {
        assert!(
            VideoGeometry {
                crop: Some(crop),
                ..Default::default()
            }
            .apply(&frame, 4, 4)
            .is_err()
        );
    }
    for scale in [[0, 2], [3, 2], [usize::MAX - 1, usize::MAX - 1]] {
        assert!(
            VideoGeometry {
                scale: Some(scale),
                ..Default::default()
            }
            .apply(&frame, 4, 4)
            .is_err()
        );
    }
}

#[test]
fn packed_rgb_keeps_pixel_components_together() {
    let frame = RawFrame::Rgb(vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
    let out = VideoGeometry {
        horizontal_flip: true,
        scale: Some([2, 1]),
        ..Default::default()
    }
    .apply(&frame, 3, 1)
    .unwrap();
    assert_eq!(out.data, [7, 8, 9, 1, 2, 3]);
    assert!(
        VideoGeometry::default()
            .apply(&RawFrame::Rgb(vec![0]), 3, 1)
            .is_err()
    );
}

#[test]
fn main10_crop_preserves_reference_samples_without_rgb_roundtrip() {
    let input = include_bytes!("fixtures/hevc/main10-ipb.mp4");
    let reference = include_bytes!("fixtures/hevc/main10-ipb.yuv");
    let mut reader = NativeReader::software(Cursor::new(input), usize::MAX).unwrap();
    let mut offset = 0;
    let mut frames = 0;
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        let [w, h] = reader.dimensions();
        let out = VideoGeometry {
            crop: Some([2, 2, w - 4, h - 4]),
            horizontal_flip: true,
            vertical_flip: true,
            scale: None,
        }
        .apply(&frame, w, h)
        .unwrap();
        let mut expected = Vec::new();
        for (pw, ph, border) in [(w, h, 2), (w / 2, h / 2, 1), (w / 2, h / 2, 1)] {
            for y in (border..ph - border).rev() {
                for x in (border..pw - border).rev() {
                    let at = offset + (y * pw + x) * 2;
                    expected.extend_from_slice(&reference[at..at + 2]);
                }
            }
            offset += pw * ph * 2;
        }
        assert_eq!(out.data, expected, "frame {frames}");
        frames += 1;
    }
    assert_eq!(frames, 17);
    assert_eq!(offset, reference.len());
}

#[test]
fn native_geometry_api_and_cli_use_owned_decoder() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4");
    let stats = fvid::native_media::decode_video_transformed(
        &path,
        None,
        &VideoGeometry {
            crop: Some([2, 2, 32, 32]),
            scale: Some([16, 8]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!((stats.width, stats.height, stats.video_frames), (16, 8, 17));
    assert_eq!(stats.pixel_format, "yuv420p10le");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            path.to_str().unwrap(),
            "--crop",
            "2:2:32:32",
            "--hflip",
            "--vflip",
            "--scale",
            "16:8",
            "--from",
            "0.04",
            "--to",
            "0.12",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["backend"], "fvid");
    assert_eq!(json["width"], 16);
    assert_eq!(json["height"], 8);
    assert_eq!(json["video_frames"], 2);
    #[cfg(feature = "media")]
    {
        let stats = fvid::media::decode_video_transformed(
            &path,
            fvid::media::DecodeTransform {
                scale: Some(fvid::media::ScaleSize {
                    width: 16,
                    height: 8,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.pixel_format, "yuv420p10le");
        assert_eq!((stats.width, stats.height), (16, 8));
    }
}
