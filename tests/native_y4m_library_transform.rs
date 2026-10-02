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
        unsharp: Some(String::new()),
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

#[test]
fn pad_compositions_match_media_geometry() {
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
                    let v = ((i * 17 + 53) & ((1usize << depth) - 1)) as u16;
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
            for transpose in [None, Some(fvid_media::TransposeMode::Clock)] {
                for scale in [
                    None,
                    Some(fvid_media::ScaleSize {
                        width: 10,
                        height: 8,
                    }),
                ] {
                    for (h, v) in [(false, false), (true, false), (false, true), (true, true)] {
                        let pad = fvid_media::PadRect {
                            width: 8,
                            height: 8,
                            x: 2,
                            y: 2,
                        };
                        let t = DecodeTransform {
                            crop: Some(CropRect {
                                x: 2,
                                y: 2,
                                width: 4,
                                height: 2,
                            }),
                            transpose,
                            scale,
                            pad: Some(pad),
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
                            transpose: transpose.map(|m| {
                                fvid::native_geometry::Transpose::parse(m.as_str()).unwrap()
                            }),
                            scale: scale.map(|s| [s.width as usize, s.height as usize]),
                            pad: Some([8, 8, 2, 2]),
                            horizontal_flip: h,
                            vertical_flip: v,
                            ..Default::default()
                        }
                        .apply_media(&frame, 8, 6)
                        .unwrap();
                        assert_eq!(
                            actual, expected.data,
                            "{chroma} {transpose:?} {scale:?} {h} {v}"
                        );
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
fn pad_known_fill_and_invalid_canvas() {
    for depth in [8u8, 10, 16] {
        for full in [false, true] {
            let chroma = if depth == 8 {
                "444".into()
            } else {
                format!("444p{depth}")
            };
            let range = if full {
                "XCOLORRANGE=FULL"
            } else {
                "XCOLORRANGE=LIMITED"
            };
            let header = fvid_media::owned_y4m::Header::parse(
                format!("YUV4MPEG2 W2 H2 C{chroma} {range}\n").as_bytes(),
            )
            .unwrap();
            let frame = vec![0; header.frame_len().unwrap()];
            let t = DecodeTransform {
                pad: Some(fvid_media::PadRect {
                    width: 4,
                    height: 4,
                    x: 2,
                    y: 2,
                }),
                ..Default::default()
            };
            let output =
                fvid_media::owned_y4m_decode::transform_frame_requested(&header, &frame, &t)
                    .unwrap();
            let step = if depth == 8 { 1 } else { 2 };
            for plane in 0..3 {
                let value = if full {
                    if plane == 0 { 0 } else { 128u16 << (depth - 8) }
                } else {
                    let code = if plane == 0 { 16u32 } else { 128 };
                    ((code * ((1u32 << depth) - 1) + 127) / 255) as u16
                };
                let bytes = value.to_le_bytes();
                for y in 0..4 {
                    for x in 0..4 {
                        let at = (plane * 16 + y * 4 + x) * step;
                        assert_eq!(
                            &output[at..at + step],
                            if x >= 2 && y >= 2 {
                                &[0, 0][..step]
                            } else {
                                &bytes[..step]
                            }
                        );
                    }
                }
            }
            for pad in [
                fvid_media::PadRect {
                    width: 4,
                    height: 4,
                    x: 3,
                    y: 0,
                },
                fvid_media::PadRect {
                    width: 4,
                    height: 4,
                    x: 4,
                    y: 0,
                },
                fvid_media::PadRect {
                    width: 0,
                    height: 4,
                    x: 0,
                    y: 0,
                },
            ] {
                let invalid = DecodeTransform {
                    pad: Some(pad),
                    ..Default::default()
                };
                assert!(
                    fvid_media::owned_y4m_decode::transform_frame_requested(
                        &header, &frame, &invalid
                    )
                    .is_err()
                );
            }
        }
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-probe-info.y4m");
    let t = DecodeTransform {
        pad: Some(fvid_media::PadRect {
            width: 32,
            height: 32,
            x: 2,
            y: 2,
        }),
        ..Default::default()
    };
    let stats = fvid_media::decode_video_transformed(&source, t).unwrap();
    assert_eq!((stats.width, stats.height, stats.video_frames), (32, 32, 3));
    assert_eq!(stats.backend, "owned Y4M planar decode");
}

#[test]
fn owned_negate_preserves_depth_and_runs_after_geometry() {
    for layout in ["420", "422", "444"] {
        for depth in [8u8, 9, 10, 12, 14, 16] {
            let chroma = if depth == 8 {
                layout.into()
            } else {
                format!("{layout}p{depth}")
            };
            let header = fvid_media::owned_y4m::Header::parse(
                format!("YUV4MPEG2 W4 H2 F30:1 C{chroma}\n").as_bytes(),
            )
            .unwrap();
            let step = if depth == 8 { 1 } else { 2 };
            let max = ((1u32 << depth) - 1) as u16;
            let input: Vec<u8> = (0..header.frame_len().unwrap() / step)
                .flat_map(|i| {
                    let v = (i * 17 % (usize::from(max) + 1)) as u16;
                    if step == 1 {
                        vec![v as u8]
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            for args in ["", "0", "1"] {
                let mut t = DecodeTransform {
                    negate: Some(args.into()),
                    horizontal_flip: true,
                    pad: Some(fvid_media::PadRect {
                        width: 8,
                        height: 4,
                        x: 2,
                        y: 2,
                    }),
                    ..Default::default()
                };
                let actual =
                    fvid_media::owned_y4m_decode::transform_frame_requested(&header, &input, &t)
                        .unwrap();
                t.negate = None;
                let geometry =
                    fvid_media::owned_y4m_decode::transform_frame_requested(&header, &input, &t)
                        .unwrap();
                let expected: Vec<u8> = geometry
                    .chunks_exact(step)
                    .flat_map(|b| {
                        let v = if step == 1 {
                            u16::from(b[0])
                        } else {
                            u16::from_le_bytes([b[0], b[1]])
                        };
                        let n = max - v;
                        if step == 1 {
                            vec![n as u8]
                        } else {
                            n.to_le_bytes().to_vec()
                        }
                    })
                    .collect();
                assert_eq!(actual, expected, "{chroma} {args}");
            }
        }
    }
    let mut malformed = vec![0, 4];
    let before = malformed.clone();
    assert!(
        fvid_media::owned_negate::Negate
            .apply(&mut malformed, 10)
            .is_err()
    );
    assert_eq!(malformed, before);
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-probe-info.y4m");
    let stats = fvid_media::decode_video_transformed(
        &source,
        DecodeTransform {
            negate: Some("1".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.backend, "owned Y4M planar decode");
    assert_eq!(stats.video_frames, 3);
}

#[test]
fn library_blur_composition_preserves_frontend_order_and_public_dispatch() {
    for depth in [8u8, 10, 16] {
        let chroma = if depth == 8 {
            "420".into()
        } else {
            format!("420p{depth}")
        };
        let header = fvid_media::owned_y4m::Header::parse(
            format!("YUV4MPEG2 W8 H6 F30:1 C{chroma}\n").as_bytes(),
        )
        .unwrap();
        let step = if depth == 8 { 1 } else { 2 };
        let input: Vec<u8> = (0..header.frame_len().unwrap() / step)
            .flat_map(|i| {
                let v = (i * 97 % (1usize << depth)) as u16;
                if step == 1 {
                    vec![v as u8]
                } else {
                    v.to_le_bytes().to_vec()
                }
            })
            .collect();
        for (avg, boxblur) in [
            (Some("2:7:1"), None),
            (None, Some("1:2")),
            (Some("1:7:2"), Some("1:2")),
        ] {
            let t = DecodeTransform {
                avgblur: avg.map(str::to_owned),
                boxblur: boxblur.map(str::to_owned),
                negate: Some("".into()),
                horizontal_flip: true,
                ..Default::default()
            };
            let actual =
                fvid_media::owned_y4m_decode::transform_frame_requested(&header, &input, &t)
                    .unwrap();
            let pixels =
                fvid_media::owned_y4m_decode::transform_frame(&header, &input, None, true, false)
                    .unwrap();
            let mut expected = fvid::native_geometry::GeometryFrame {
                width: 8,
                height: 6,
                subsampling: Some([2, 2]),
                data: pixels,
            };
            fvid::native_pixels::PixelFilters::from_request(&t)
                .unwrap()
                .apply(&mut expected, depth)
                .unwrap();
            assert_eq!(actual, expected.data);
        }
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-probe-info.y4m");
    for t in [
        DecodeTransform {
            avgblur: Some("".into()),
            ..Default::default()
        },
        DecodeTransform {
            boxblur: Some("1:2".into()),
            ..Default::default()
        },
    ] {
        let stats = fvid_media::decode_video_transformed(&source, t).unwrap();
        assert_eq!(stats.backend, "owned Y4M planar decode");
        assert_eq!(stats.video_frames, 3);
    }
}

#[test]
fn pixelize_and_chroma_shift_compositions_use_shared_library_kernels() {
    for layout in ["420", "422", "444"] {
        for depth in [8u8, 10, 16] {
            let chroma = if depth == 8 {
                layout.into()
            } else {
                format!("{layout}p{depth}")
            };
            let header = fvid_media::owned_y4m::Header::parse(
                format!("YUV4MPEG2 W8 H6 F30:1 C{chroma}\n").as_bytes(),
            )
            .unwrap();
            let step = if depth == 8 { 1 } else { 2 };
            let input: Vec<u8> = (0..header.frame_len().unwrap() / step)
                .flat_map(|i| {
                    let v = (i * 131 % (1usize << depth)) as u16;
                    if step == 1 {
                        vec![v as u8]
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            for (pixelize, shift) in [
                (Some("2:3"), None),
                (None, Some("cbh=1:crv=-1:edge=wrap")),
                (Some("2:2"), Some("cbh=-2:crv=1")),
            ] {
                let t = DecodeTransform {
                    pixelize: pixelize.map(str::to_owned),
                    chromashift: shift.map(str::to_owned),
                    negate: Some("".into()),
                    horizontal_flip: true,
                    ..Default::default()
                };
                let actual =
                    fvid_media::owned_y4m_decode::transform_frame_requested(&header, &input, &t)
                        .unwrap();
                let pixels = fvid_media::owned_y4m_decode::transform_frame(
                    &header, &input, None, true, false,
                )
                .unwrap();
                let subsampling = match layout {
                    "420" => [2, 2],
                    "422" => [2, 1],
                    _ => [1, 1],
                };
                let mut expected = fvid::native_geometry::GeometryFrame {
                    width: 8,
                    height: 6,
                    subsampling: Some(subsampling),
                    data: pixels,
                };
                fvid::native_pixels::PixelFilters::from_request(&t)
                    .unwrap()
                    .apply(&mut expected, depth)
                    .unwrap();
                assert_eq!(actual, expected.data, "{chroma} {pixelize:?} {shift:?}");
            }
        }
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-probe-info.y4m");
    for t in [
        DecodeTransform {
            pixelize: Some("2:2".into()),
            ..Default::default()
        },
        DecodeTransform {
            chromashift: Some("cbh=1".into()),
            ..Default::default()
        },
    ] {
        let stats = fvid_media::decode_video_transformed(&source, t).unwrap();
        assert_eq!(stats.backend, "owned Y4M planar decode");
        assert_eq!(stats.video_frames, 3);
    }
}

#[test]
fn y4m_plane_shuffle_preserves_or_promotes_chroma_and_matches_frontend() {
    for layout in ["420", "422", "444"] {
        for depth in [8u8, 10, 16] {
            let chroma = if depth == 8 {
                layout.into()
            } else {
                format!("{layout}p{depth}")
            };
            let text = format!("YUV4MPEG2 W8 H6 F30:1 C{chroma}\n");
            let header = fvid_media::owned_y4m::Header::parse(text.as_bytes()).unwrap();
            let step = if depth == 8 { 1 } else { 2 };
            let pixels: Vec<u8> = (0..header.frame_len().unwrap() / step)
                .flat_map(|i| {
                    let v = (i * 113 % (1usize << depth)) as u16;
                    if step == 1 {
                        vec![v as u8]
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            let source = [text.as_bytes(), b"FRAME\n", &pixels].concat();
            for a in 0..3 {
                for b in 0..3 {
                    for c in 0..3 {
                        let t = DecodeTransform {
                            shuffleplanes: Some(format!("{a}:{b}:{c}")),
                            negate: Some("".into()),
                            horizontal_flip: true,
                            ..Default::default()
                        };
                        let actual = fvid_media::owned_y4m_decode::transform_frame_requested(
                            &header, &pixels, &t,
                        )
                        .unwrap();
                        let data = fvid_media::owned_y4m_decode::transform_frame(
                            &header, &pixels, None, true, false,
                        )
                        .unwrap();
                        let subsampling = match layout {
                            "420" => [2, 2],
                            "422" => [2, 1],
                            _ => [1, 1],
                        };
                        let mut expected = fvid::native_geometry::GeometryFrame {
                            width: 8,
                            height: 6,
                            subsampling: Some(subsampling),
                            data,
                        };
                        fvid::native_pixels::PixelFilters::from_request(&t)
                            .unwrap()
                            .apply(&mut expected, depth)
                            .unwrap();
                        assert_eq!(actual, expected.data, "{chroma} {a}:{b}:{c}");
                        let stats = fvid_media::owned_y4m_decode::decode_reader_transformed(
                            Cursor::new(&source),
                            &t,
                        )
                        .unwrap();
                        let output_layout = if layout == "444" || a != 0 || b == 0 || c == 0 {
                            "444"
                        } else {
                            layout
                        };
                        let format = if depth == 8 {
                            format!("yuv{output_layout}p")
                        } else {
                            format!("yuv{output_layout}p{depth}le")
                        };
                        assert_eq!(stats.pixel_format, format);
                    }
                }
            }
        }
    }
    let header = fvid_media::owned_y4m::Header::parse(b"YUV4MPEG2 W2 H2 C420\n").unwrap();
    let t = DecodeTransform {
        shuffleplanes: Some("1:0:2".into()),
        ..Default::default()
    };
    assert_eq!(
        fvid_media::owned_y4m_decode::transform_frame_requested(&header, &[1, 2, 3, 4, 5, 6], &t)
            .unwrap(),
        vec![5, 5, 5, 5, 1, 2, 3, 4, 6, 6, 6, 6]
    );
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-probe-info.y4m");
    let stats = fvid_media::decode_video_transformed(&source, t).unwrap();
    assert_eq!(stats.pixel_format, "yuv444p");
    assert_eq!(stats.backend, "owned Y4M planar decode");
}

#[test]
fn owned_y4m_gradients_match_frontend_and_preserve_filter_order() {
    for layout in ["420", "422", "444"] {
        for depth in [8u8, 10, 16] {
            let chroma = if depth == 8 {
                layout.into()
            } else {
                format!("{layout}p{depth}")
            };
            let header = fvid_media::owned_y4m::Header::parse(
                format!("YUV4MPEG2 W8 H6 F30:1 C{chroma}\n").as_bytes(),
            )
            .unwrap();
            let step = if depth == 8 { 1 } else { 2 };
            let input: Vec<u8> = (0..header.frame_len().unwrap() / step)
                .flat_map(|i| {
                    let v = (i * 173 % (1usize << depth)) as u16;
                    if step == 1 {
                        vec![v as u8]
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            for op in 0..6 {
                let args = Some("planes=7:scale=0.5:delta=2".into());
                let mut t = DecodeTransform {
                    negate: Some("".into()),
                    pixelize: Some("2:2".into()),
                    horizontal_flip: true,
                    ..Default::default()
                };
                match op {
                    0 => t.sobel = args,
                    1 => t.prewitt = args,
                    2 => t.roberts = args,
                    3 => t.kirsch = args,
                    4 => t.scharr = args,
                    _ => {
                        t.sobel = args.clone();
                        t.prewitt = args.clone();
                        t.roberts = args.clone();
                        t.kirsch = args.clone();
                        t.scharr = args;
                    }
                }
                let actual =
                    fvid_media::owned_y4m_decode::transform_frame_requested(&header, &input, &t)
                        .unwrap();
                let data = fvid_media::owned_y4m_decode::transform_frame(
                    &header, &input, None, true, false,
                )
                .unwrap();
                let sub = match layout {
                    "420" => [2, 2],
                    "422" => [2, 1],
                    _ => [1, 1],
                };
                let mut expected = fvid::native_geometry::GeometryFrame {
                    width: 8,
                    height: 6,
                    subsampling: Some(sub),
                    data,
                };
                fvid::native_pixels::PixelFilters::from_request(&t)
                    .unwrap()
                    .apply(&mut expected, depth)
                    .unwrap();
                assert_eq!(actual, expected.data, "{chroma} {op}");
            }
        }
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-probe-info.y4m");
    for op in 0..5 {
        let mut t = DecodeTransform::default();
        let args = Some("".into());
        match op {
            0 => t.sobel = args,
            1 => t.prewitt = args,
            2 => t.roberts = args,
            3 => t.kirsch = args,
            _ => t.scharr = args,
        }
        let stats = fvid_media::decode_video_transformed(&source, t).unwrap();
        assert_eq!(stats.backend, "owned Y4M planar decode");
        assert_eq!(stats.video_frames, 3);
    }
}

#[test]
fn y4m_morphology_matches_frontend_and_keeps_canonical_order() {
    for layout in ["420", "422", "444"] {
        for depth in [8u8, 10, 16] {
            let chroma = if depth == 8 {
                layout.into()
            } else {
                format!("{layout}p{depth}")
            };
            let header = fvid_media::owned_y4m::Header::parse(
                format!("YUV4MPEG2 W8 H6 F30:1 C{chroma}\n").as_bytes(),
            )
            .unwrap();
            let step = if depth == 8 { 1 } else { 2 };
            let input: Vec<u8> = (0..header.frame_len().unwrap() / step)
                .flat_map(|i| {
                    let v = (i * 193 % (1usize << depth)) as u16;
                    if step == 1 {
                        vec![v as u8]
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            for op in 0..3 {
                let mut t = DecodeTransform {
                    negate: Some("".into()),
                    pixelize: Some("2:2".into()),
                    chromashift: Some("cbh=1".into()),
                    horizontal_flip: true,
                    ..Default::default()
                };
                if op != 1 {
                    t.dilation = Some("".into());
                }
                if op != 0 {
                    t.erosion = Some("".into());
                }
                let actual =
                    fvid_media::owned_y4m_decode::transform_frame_requested(&header, &input, &t)
                        .unwrap();
                let data = fvid_media::owned_y4m_decode::transform_frame(
                    &header, &input, None, true, false,
                )
                .unwrap();
                let sub = match layout {
                    "420" => [2, 2],
                    "422" => [2, 1],
                    _ => [1, 1],
                };
                let mut expected = fvid::native_geometry::GeometryFrame {
                    width: 8,
                    height: 6,
                    subsampling: Some(sub),
                    data,
                };
                fvid::native_pixels::PixelFilters::from_request(&t)
                    .unwrap()
                    .apply(&mut expected, depth)
                    .unwrap();
                assert_eq!(actual, expected.data, "{chroma} {op}");
            }
        }
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-probe-info.y4m");
    for t in [
        DecodeTransform {
            dilation: Some("".into()),
            ..Default::default()
        },
        DecodeTransform {
            erosion: Some("".into()),
            ..Default::default()
        },
    ] {
        let stats = fvid_media::decode_video_transformed(&source, t).unwrap();
        assert_eq!(stats.backend, "owned Y4M planar decode");
        assert_eq!(stats.video_frames, 3);
    }
}

#[test]
fn vertical_chroma_fixture_decodes_and_transposes_back_to_422() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-vertical-chroma-10.y4m");
    let info = fvid_media::probe(&source).unwrap();
    assert_eq!(info.streams[0].duration, Some(2));
    let plain = fvid_media::decode_video(&source).unwrap();
    assert_eq!((plain.width, plain.height, plain.video_frames), (4, 2, 2));
    assert_eq!(plain.pixel_format, "yuv440p10le");
    let t = DecodeTransform {
        transpose: Some(fvid_media::TransposeMode::Clock),
        ..Default::default()
    };
    let transformed = fvid_media::decode_video_transformed(&source, t.clone()).unwrap();
    assert_eq!(
        (
            transformed.width,
            transformed.height,
            transformed.video_frames
        ),
        (2, 4, 2)
    );
    assert_eq!(transformed.pixel_format, "yuv422p10le");
    let bytes = std::fs::read(&source).unwrap();
    let mut lines = bytes.splitn(3, |&b| b == b'\n');
    let header = fvid_media::owned_y4m::Header::parse(lines.next().unwrap()).unwrap();
    lines.next().unwrap();
    let payload = lines.next().unwrap();
    let pixels =
        fvid_media::owned_y4m_decode::transform_frame_requested(&header, &payload[..32], &t)
            .unwrap();
    let indices = [4u16, 0, 5, 1, 6, 2, 7, 3, 8, 9, 10, 11, 12, 13, 14, 15];
    let expected: Vec<u8> = indices
        .into_iter()
        .flat_map(|i| (i * 17).to_le_bytes())
        .collect();
    assert_eq!(pixels, expected);
}
