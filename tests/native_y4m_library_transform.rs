use fvid_media::{CropRect, DecodeTransform};
use std::{io::Cursor, path::Path};
#[test]
fn owned_plane_pixels_match_frontend_cpu_for_all_chroma_depth_and_reflections() {
    for layout in ["420", "422", "444"] {
        for depth in [8u8, 10, 16] {
            let chroma = if depth == 8 {
                layout.into()
            } else {
                format!("{layout}p{depth}")
            };
            let text = format!("YUV4MPEG2 W8 H6 F30000:1001 Ip C{chroma}\n");
            let header = fvid_media::owned_y4m::Header::parse(text.as_bytes()).unwrap();
            let step = if depth == 8 { 1 } else { 2 };
            let mut pixels = Vec::new();
            for sample in 0..header.frame_len().unwrap() / step {
                let value = ((sample * 17 + 257) & ((1usize << depth) - 1)) as u16;
                if step == 1 {
                    pixels.push(value as u8);
                } else {
                    pixels.extend(value.to_le_bytes());
                }
            }
            let source = [text.as_bytes(), b"FRAME\n", &pixels].concat();
            for crop in [
                None,
                Some(CropRect {
                    x: 2,
                    y: 2,
                    width: 4,
                    height: 2,
                }),
            ] {
                for (h, v) in [(false, false), (true, false), (false, true), (true, true)] {
                    let actual =
                        fvid_media::owned_y4m_decode::transform_frame(&header, &pixels, crop, h, v)
                            .unwrap();
                    let expected_pixels = if depth == 8 {
                        let mut expected = Vec::new();
                        fvid::process_with_options(
                            Cursor::new(&source),
                            &mut expected,
                            fvid::Transform {
                                crop: crop.map(|c| fvid::Crop {
                                    x: c.x,
                                    y: c.y,
                                    width: c.width,
                                    height: c.height,
                                }),
                                horizontal: h,
                                vertical: v,
                            },
                            64 << 20,
                            fvid::ExecutionOptions {
                                backend: fvid::Backend::Cpu,
                                device: 0,
                            },
                        )
                        .unwrap();
                        let payload = expected.splitn(3, |&b| b == b'\n').nth(2).unwrap();
                        payload.to_vec()
                    } else {
                        let mut reader = fvid::playback_native::NativeReader::software(
                            Cursor::new(&source),
                            64 << 20,
                        )
                        .unwrap();
                        let frame = reader.read_frame_raw().unwrap().unwrap();
                        fvid::native_geometry::VideoGeometry {
                            crop: crop.map(|c| [c.x, c.y, c.width, c.height]),
                            horizontal_flip: h,
                            vertical_flip: v,
                            ..Default::default()
                        }
                        .apply(&frame, 8, 6)
                        .unwrap()
                        .data
                    };
                    assert_eq!(actual, expected_pixels, "{chroma} {h} {v}");
                }
            }
        }
    }
}
#[test]
fn known_pixels_preserve_multibyte_samples_in_all_planes() {
    for depth in [8, 16] {
        let format = if depth == 8 { "420" } else { "420p16" };
        let header =
            fvid_media::owned_y4m::Header::parse(format!("YUV4MPEG2 W4 H2 C{format}\n").as_bytes())
                .unwrap();
        let values: Vec<u16> = (0..12)
            .map(|i| if depth == 8 { i } else { i + 256 })
            .collect();
        let bytes: Vec<u8> = if depth == 8 {
            values.iter().map(|&v| v as u8).collect()
        } else {
            values.iter().flat_map(|v| v.to_le_bytes()).collect()
        };
        let actual = fvid_media::owned_y4m_decode::transform_frame(
            &header,
            &bytes,
            Some(CropRect {
                x: 2,
                y: 0,
                width: 2,
                height: 2,
            }),
            true,
            true,
        )
        .unwrap();
        let ordered = [7usize, 6, 3, 2, 9, 11];
        let expected: Vec<u8> = if depth == 8 {
            ordered.iter().map(|&i| values[i] as u8).collect()
        } else {
            ordered
                .iter()
                .flat_map(|&i| values[i].to_le_bytes())
                .collect()
        };
        assert_eq!(actual, expected);
    }
}
#[test]
fn interval_uses_exact_frame_clock_and_stops_before_unrequested_tail() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-probe-info.y4m");
    let data = std::fs::read(&source).unwrap();
    for (from, to, count) in [(33333, 66667, 2), (33334, 66666, 0), (0, 33333, 1)] {
        let transform = DecodeTransform {
            interval: Some((from, to)),
            crop: Some(CropRect {
                x: 2,
                y: 2,
                width: 8,
                height: 8,
            }),
            horizontal_flip: true,
            ..Default::default()
        };
        let stats = fvid_media::decode_video_transformed(&source, transform).unwrap();
        assert_eq!(
            (stats.video_frames, stats.width, stats.height),
            (count, 8, 8)
        );
        assert_eq!(stats.backend, "owned Y4M planar decode");
    }
    let mut bad = data.clone();
    bad.extend_from_slice(b"not a frame\n");
    let interval = DecodeTransform {
        interval: Some((0, 90000)),
        ..Default::default()
    };
    assert_eq!(
        fvid_media::owned_y4m_decode::decode_reader_transformed(Cursor::new(&bad), &interval)
            .unwrap()
            .video_frames,
        3
    );
    assert!(fvid_media::owned_y4m_decode::decode_reader(Cursor::new(&bad)).is_err());
    let unsupported = DecodeTransform {
        negate: Some(String::new()),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_y4m_decode::decode_reader_transformed(Cursor::new(&data), &unsupported)
            .unwrap_err()
            .contains("transform options")
    );
}

#[test]
fn scale_pixels_match_media_sampling_with_crop_and_reflections() {
    for layout in ["420", "422", "444"] {
        for depth in [8u8, 10, 16] {
            let chroma = if depth == 8 {
                layout.into()
            } else {
                format!("{layout}p{depth}")
            };
            let text = format!("YUV4MPEG2 W8 H6 F30:1 Ip C{chroma}\n");
            let header = fvid_media::owned_y4m::Header::parse(text.as_bytes()).unwrap();
            let step = if depth == 8 { 1 } else { 2 };
            let pixels: Vec<u8> = (0..header.frame_len().unwrap() / step)
                .flat_map(|i| {
                    let value = ((i * 37 + 123) & ((1usize << depth) - 1)) as u16;
                    if step == 1 {
                        vec![value as u8]
                    } else {
                        value.to_le_bytes().to_vec()
                    }
                })
                .collect();
            let source = [text.as_bytes(), b"FRAME\n", &pixels].concat();
            let mut reader =
                fvid::playback_native::NativeReader::software(Cursor::new(&source), 64 << 20)
                    .unwrap();
            let frame = reader.read_frame_raw().unwrap().unwrap();
            for (w, h) in [(2, 2), (6, 4), (12, 10)] {
                for (horizontal, vertical) in
                    [(false, false), (true, false), (false, true), (true, true)]
                {
                    let transform = DecodeTransform {
                        crop: Some(CropRect {
                            x: 2,
                            y: 2,
                            width: 4,
                            height: 2,
                        }),
                        horizontal_flip: horizontal,
                        vertical_flip: vertical,
                        scale: Some(fvid_media::ScaleSize {
                            width: w,
                            height: h,
                        }),
                        ..Default::default()
                    };
                    let actual = fvid_media::owned_y4m_decode::transform_frame_requested(
                        &header, &pixels, &transform,
                    )
                    .unwrap();
                    let expected = fvid::native_geometry::VideoGeometry {
                        crop: Some([2, 2, 4, 2]),
                        horizontal_flip: horizontal,
                        vertical_flip: vertical,
                        scale: Some([w as usize, h as usize]),
                        ..Default::default()
                    }
                    .apply_media(&frame, 8, 6)
                    .unwrap();
                    assert_eq!(
                        actual, expected.data,
                        "{chroma} {w}x{h} {horizontal} {vertical}"
                    );
                    let stats = fvid_media::owned_y4m_decode::decode_reader_transformed(
                        Cursor::new(&source),
                        &transform,
                    )
                    .unwrap();
                    assert_eq!((stats.width, stats.height, stats.video_frames), (w, h, 1));
                }
            }
            for (w, h) in [(0, 2), (3, 2), (8194, 2), (2, 4322)] {
                let transform = DecodeTransform {
                    scale: Some(fvid_media::ScaleSize {
                        width: w,
                        height: h,
                    }),
                    ..Default::default()
                };
                assert!(
                    fvid_media::owned_y4m_decode::transform_frame_requested(
                        &header, &pixels, &transform
                    )
                    .is_err()
                );
            }
        }
    }
}

#[test]
fn scale_known_samples_and_public_dispatch() {
    let header = fvid_media::owned_y4m::Header::parse(b"YUV4MPEG2 W4 H2 C444\n").unwrap();
    let pixels: Vec<u8> = (0..24).collect();
    let transform = DecodeTransform {
        scale: Some(fvid_media::ScaleSize {
            width: 2,
            height: 2,
        }),
        ..Default::default()
    };
    let actual =
        fvid_media::owned_y4m_decode::transform_frame_requested(&header, &pixels, &transform)
            .unwrap();
    assert_eq!(actual, vec![1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23]);
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-probe-info.y4m");
    let stats = fvid_media::decode_video_transformed(&source, transform).unwrap();
    assert_eq!((stats.width, stats.height, stats.video_frames), (2, 2, 3));
    assert_eq!(stats.backend, "owned Y4M planar decode");
}

#[test]
fn transpose_compositions_match_sample_planes() {
    use fvid_media::TransposeMode;
    for layout in ["420", "422", "444"] {
        for depth in [8u8, 10, 16] {
            let chroma = if depth == 8 {
                layout.into()
            } else {
                format!("{layout}p{depth}")
            };
            let text = format!("YUV4MPEG2 W8 H6 F30:1 Ip C{chroma}\n");
            let header = fvid_media::owned_y4m::Header::parse(text.as_bytes()).unwrap();
            let step = if depth == 8 { 1 } else { 2 };
            let pixels: Vec<u8> = (0..header.frame_len().unwrap() / step)
                .flat_map(|i| {
                    let v = ((i * 43 + 91) & ((1usize << depth) - 1)) as u16;
                    if step == 1 {
                        vec![v as u8]
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            let source = [text.as_bytes(), b"FRAME\n", &pixels].concat();
            let mut reader =
                fvid::playback_native::NativeReader::software(Cursor::new(&source), 64 << 20)
                    .unwrap();
            let frame = reader.read_frame_raw().unwrap().unwrap();
            for mode in [
                TransposeMode::Clock,
                TransposeMode::CClock,
                TransposeMode::ClockFlip,
                TransposeMode::CClockFlip,
            ] {
                for scale in [
                    None,
                    Some(fvid_media::ScaleSize {
                        width: 6,
                        height: 8,
                    }),
                ] {
                    for (h, v) in [(false, false), (true, false), (false, true), (true, true)] {
                        let t = DecodeTransform {
                            crop: Some(CropRect {
                                x: 2,
                                y: 2,
                                width: 4,
                                height: 2,
                            }),
                            transpose: Some(mode),
                            scale,
                            horizontal_flip: h,
                            vertical_flip: v,
                            ..Default::default()
                        };
                        let actual = fvid_media::owned_y4m_decode::transform_frame_requested(
                            &header, &pixels, &t,
                        )
                        .unwrap();
                        let expected = fvid::native_geometry::VideoGeometry {
                            crop: Some([2, 2, 4, 2]),
                            transpose: Some(
                                fvid::native_geometry::Transpose::parse(mode.as_str()).unwrap(),
                            ),
                            scale: scale.map(|s| [s.width as usize, s.height as usize]),
                            horizontal_flip: h,
                            vertical_flip: v,
                            ..Default::default()
                        }
                        .apply_media(&frame, 8, 6)
                        .unwrap();
                        assert_eq!(actual, expected.data, "{chroma} {mode:?} {h} {v}");
                        let stats = fvid_media::owned_y4m_decode::decode_reader_transformed(
                            Cursor::new(&source),
                            &t,
                        )
                        .unwrap();
                        assert_eq!(
                            (stats.width as usize, stats.height as usize),
                            (expected.width, expected.height)
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn transpose_known_pixels_and_output_chroma_are_exact() {
    use fvid_media::TransposeMode;
    let header = fvid_media::owned_y4m::Header::parse(b"YUV4MPEG2 W4 H2 C444\n").unwrap();
    let pixels: Vec<u8> = (0..24).collect();
    for (mode, order) in [
        (TransposeMode::Clock, [4, 0, 5, 1, 6, 2, 7, 3]),
        (TransposeMode::CClock, [3, 7, 2, 6, 1, 5, 0, 4]),
        (TransposeMode::ClockFlip, [7, 3, 6, 2, 5, 1, 4, 0]),
        (TransposeMode::CClockFlip, [0, 4, 1, 5, 2, 6, 3, 7]),
    ] {
        let t = DecodeTransform {
            transpose: Some(mode),
            ..Default::default()
        };
        let actual =
            fvid_media::owned_y4m_decode::transform_frame_requested(&header, &pixels, &t).unwrap();
        let expected: Vec<u8> = (0..3).flat_map(|p| order.map(|i| i + p * 8)).collect();
        assert_eq!(actual, expected);
    }
    let header = fvid_media::owned_y4m::Header::parse(b"YUV4MPEG2 W4 H2 C422\n").unwrap();
    let t = DecodeTransform {
        transpose: Some(TransposeMode::Clock),
        ..Default::default()
    };
    let bytes = [b"YUV4MPEG2 W4 H2 C422\nFRAME\n".as_slice(), &[0; 16]].concat();
    let stats =
        fvid_media::owned_y4m_decode::decode_reader_transformed(Cursor::new(bytes), &t).unwrap();
    assert_eq!((stats.width, stats.height), (2, 4));
    assert_eq!(stats.pixel_format, "yuv440p");
    assert_eq!(
        fvid_media::owned_y4m_decode::transform_frame_requested(&header, &[0; 16], &t)
            .unwrap()
            .len(),
        16
    );
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-probe-info.y4m");
    let stats = fvid_media::decode_video_transformed(&source, t).unwrap();
    assert_eq!(stats.backend, "owned Y4M planar decode");
    assert_eq!(stats.video_frames, 3);
}
