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
        ..Default::default()
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
            ..Default::default()
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

#[test]
fn all_quarter_turns_match_reference_and_main10_padding_is_neutral() {
    use fvid::{
        codec::avc_picture::IntraPicture, native_geometry::Transpose, playback_native::AvcColour,
    };
    let frame = RawFrame::Avc {
        picture: std::sync::Arc::new(IntraPicture {
            coded_width: 8,
            coded_height: 4,
            crop: [0; 4],
            bit_depth: 10,
            y: (0..32).map(|i| 64 + 13 * i).collect(),
            cb: (0..8).map(|i| 500 + 7 * i).collect(),
            cr: (0..8).map(|i| 600 - 9 * i).collect(),
        }),
        colour: AvcColour::default(),
    };
    for (mode, reference) in [
        (
            Transpose::Clock,
            include_bytes!("fixtures/geometry/clock.yuv").as_slice(),
        ),
        (
            Transpose::CClock,
            include_bytes!("fixtures/geometry/cclock.yuv").as_slice(),
        ),
        (
            Transpose::ClockFlip,
            include_bytes!("fixtures/geometry/clock_flip.yuv").as_slice(),
        ),
        (
            Transpose::CClockFlip,
            include_bytes!("fixtures/geometry/cclock_flip.yuv").as_slice(),
        ),
    ] {
        let geometry = VideoGeometry {
            transpose: Some(mode),
            pad: Some([8, 12, 2, 2]),
            ..Default::default()
        };
        let out = geometry.apply(&frame, 8, 4).unwrap();
        assert_eq!((out.width, out.height), (8, 12));
        let mut inside = Vec::new();
        let mut offset = 0;
        for (pw, ph, bx, by, iw, ih, black) in [
            (8, 12, 2, 2, 4, 8, 64u16),
            (4, 6, 1, 1, 2, 4, 512),
            (4, 6, 1, 1, 2, 4, 512),
        ] {
            for y in 0..ph {
                for x in 0..pw {
                    let at = offset + (y * pw + x) * 2;
                    let bytes = &out.data[at..at + 2];
                    if (bx..bx + iw).contains(&x) && (by..by + ih).contains(&y) {
                        inside.extend_from_slice(bytes);
                    } else {
                        assert_eq!(bytes, black.to_le_bytes(), "{mode:?} padding at {x},{y}");
                    }
                }
            }
            offset += pw * ph * 2;
        }
        assert_eq!(inside, reference, "{mode:?}");
    }
}

#[test]
fn full_range_padding_and_operation_order_keep_black_and_neutral_chroma() {
    use fvid::{
        native_geometry::Transpose,
        playback_native::{AvcColour, Planar8},
    };
    let frame = RawFrame::Planar8(std::sync::Arc::new(Planar8 {
        width: 4,
        height: 4,
        chroma_width: 2,
        chroma_height: 2,
        y: (0..16).collect(),
        cb: vec![40, 41, 42, 43],
        cr: vec![80, 81, 82, 83],
        colour: AvcColour {
            full: true,
            ..Default::default()
        },
    }));
    let geometry = VideoGeometry {
        crop: Some([2, 0, 2, 4]),
        horizontal_flip: true,
        vertical_flip: false,
        transpose: Some(Transpose::Clock),
        rotate: None,
        pad: Some([8, 4, 2, 2]),
        scale: Some([4, 2]),
    };
    let out = geometry.apply(&frame, 4, 4).unwrap();
    // crop -> horizontal flip -> clockwise turn -> black canvas -> centre sampling.
    assert_eq!(out.data, [0, 0, 0, 0, 0, 10, 2, 0, 43, 128, 83, 128]);
    for pad in [
        [2, 2, 0, 0],
        [8, 8, 1, 0],
        [8, 8, 0, 1],
        [8, 8, usize::MAX, 0],
    ] {
        assert!(
            VideoGeometry {
                pad: Some(pad),
                ..Default::default()
            }
            .apply(&frame, 4, 4)
            .is_err()
        );
    }
    assert!(Transpose::parse("invalid").is_err());
}

#[test]
fn transpose_padding_cli_runs_without_media_feature() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            path.to_str().unwrap(),
            "--crop",
            "0:0:32:16",
            "--transpose",
            "clock",
            "--pad",
            "24:40:4:4",
            "--scale",
            "12:20",
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
    assert_eq!(json["width"], 12);
    assert_eq!(json["height"], 20);
    assert_eq!(json["pixel_format"], "yuv420p10le");
    assert_eq!(json["video_frames"], 17);
    {
        let stats = fvid::media::decode_video_transformed(
            &path,
            fvid::media::DecodeTransform {
                crop: Some(fvid::media::CropRect {
                    x: 0,
                    y: 0,
                    width: 32,
                    height: 16,
                }),
                transpose: Some(fvid::media::TransposeMode::Clock),
                pad: Some(fvid::media::PadRect {
                    width: 24,
                    height: 40,
                    x: 4,
                    y: 4,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!((stats.width, stats.height), (24, 40));
    }
}

#[test]
fn quarter_turn_swaps_422_chroma_axes_without_resampling() {
    use fvid::{
        native_geometry::Transpose,
        playback_native::{AvcColour, Planar8},
    };
    let frame = RawFrame::Planar8(std::sync::Arc::new(Planar8 {
        width: 4,
        height: 2,
        chroma_width: 2,
        chroma_height: 2,
        y: vec![0, 1, 2, 3, 4, 5, 6, 7],
        cb: vec![10, 11, 12, 13],
        cr: vec![20, 21, 22, 23],
        colour: AvcColour::default(),
    }));
    let out = VideoGeometry {
        transpose: Some(Transpose::Clock),
        ..Default::default()
    }
    .apply(&frame, 4, 2)
    .unwrap();
    assert_eq!(
        (out.width, out.height, out.subsampling),
        (2, 4, Some([1, 2]))
    );
    assert_eq!(
        out.data,
        [4, 0, 5, 1, 6, 2, 7, 3, 12, 10, 13, 11, 22, 20, 23, 21]
    );
}

#[test]
fn decoded_statistics_name_the_transposed_422_layout() {
    use fvid::native_geometry::Transpose;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/geometry/422.y4m");
    let stats = fvid::native_media::decode_video_transformed(
        &path,
        None,
        &VideoGeometry {
            transpose: Some(Transpose::Clock),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!((stats.width, stats.height, stats.video_frames), (2, 4, 1));
    assert_eq!(stats.pixel_format, "yuv440p");
}

#[test]
fn rgb_padding_is_zero_without_splitting_colour_triplets() {
    use fvid::native_geometry::Transpose;
    let out = VideoGeometry {
        transpose: Some(Transpose::Clock),
        pad: Some([3, 4, 1, 1]),
        ..Default::default()
    }
    .apply(&RawFrame::Rgb(vec![1, 2, 3, 4, 5, 6]), 2, 1)
    .unwrap();
    assert_eq!((out.width, out.height, out.subsampling), (3, 4, None));
    let mut expected = vec![0; 36];
    expected[12..15].copy_from_slice(&[1, 2, 3]);
    expected[21..24].copy_from_slice(&[4, 5, 6]);
    assert_eq!(out.data, expected);
}

#[test]
fn media_sampling_preserves_the_native_contract_separately() {
    let input = RawFrame::Rgb((0..16u8).flat_map(|x| [x, x, x]).collect());
    let geometry = VideoGeometry {
        scale: Some([12, 1]),
        ..Default::default()
    };
    let media = geometry.apply_media(&input, 16, 1).unwrap();
    let native = geometry.apply(&input, 16, 1).unwrap();
    assert_eq!(
        media.data.as_chunks::<3>().0.iter().map(|p| p[0]).collect::<Vec<_>>(),
        [0, 1, 3, 4, 5, 7, 8, 9, 11, 12, 13, 15]
    );
    assert_eq!(
        native
            .data
            .as_chunks::<3>().0.iter()
            .map(|p| p[0])
            .collect::<Vec<_>>(),
        [0, 2, 3, 4, 6, 7, 8, 10, 11, 12, 14, 15]
    );
}

#[test]
fn stored_crop_is_applied_before_user_geometry_in_display_coordinates() {
    let frame = RawFrame::Rgb((0..18).collect()); // 3x2 distinct RGB pixels
    let geometry = VideoGeometry { horizontal_flip: true, ..Default::default() };
    let out = geometry.apply_cropped_display(&frame, 2, 3, 90, [0, 1, 0, 0]).unwrap();
    assert_eq!((out.width, out.height), (2, 2));
    // Clockwise rows: [9,0], [12,3], [15,6]; discard first then flip.
    assert_eq!(out.data, [3,4,5,12,13,14,6,7,8,15,16,17]);
    assert!(geometry.apply_cropped_display(&frame, 2, 3, 90, [2,0,0,0]).is_err());
}

#[test]
fn transformed_decode_uses_the_stored_visible_area() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/display/crops.mkv");
    let mut reader = NativeReader::software(std::io::BufReader::new(std::fs::File::open(&source).unwrap()), usize::MAX).unwrap();
    reader.read_frame_raw().unwrap().unwrap();
    let [w,h] = reader.dimensions();
    let [l,t,r,b] = reader.insets();
    assert_ne!([l,t,r,b], [0;4]);
    let stats = fvid::native_media::decode_video_transformed(&source, None,
        &VideoGeometry { horizontal_flip: true, ..Default::default() }).unwrap();
    assert_eq!((stats.width as usize, stats.height as usize), (w-(l+r) as usize,h-(t+b) as usize));
    assert!(stats.video_frames > 0);
}

#[test]
fn rotation_runs_between_transpose_and_padding() {
    let frame = fvid::playback_native::RawFrame::Rgb(vec![10,20,30,40,50,60]);
    let geometry = VideoGeometry {
        transpose: Some(fvid::native_geometry::Transpose::Clock),
        rotate: Some(fvid::media_info::RotateAngle { degrees: 90.0 }),
        pad: Some([4,1,1,0]),
        ..Default::default()
    };
    let output = geometry.apply(&frame,2,1).unwrap();
    assert_eq!((output.width, output.height),(4,1));
    assert_eq!(output.data,vec![0,0,0,40,50,60,10,20,30,0,0,0]);
}
