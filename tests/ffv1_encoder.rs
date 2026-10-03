use fvid_media::owned_ffv1_encoder as ffv1_encoder;
use fvid::{
    container::matroska_write::{Encoding, PacketWriter, TrackSpec},
    native_geometry::GeometryFrame,
};
use std::io::Cursor;
fn image(
    width: usize,
    height: usize,
    sx: usize,
    sy: usize,
    depth: u8,
    pattern: usize,
) -> GeometryFrame {
    let count = width * height + 2 * width.div_ceil(sx) * height.div_ceil(sy);
    let max = (1u32 << depth) - 1;
    let data = (0..count)
        .flat_map(|i| {
            let value = match pattern {
                0 => 0,
                1 => max,
                2 => (i as u32 * 97) & max,
                _ => ((i * i * 137 + i * 73 + pattern * 319) as u32) & max,
            } as u16;
            if depth == 8 {
                vec![value as u8]
            } else {
                value.to_le_bytes().to_vec()
            }
        })
        .collect();
    GeometryFrame {
        width,
        height,
        subsampling: Some([sx, sy]),
        data,
    }
}
fn mux(images: &[GeometryFrame], depth: u8) -> Vec<u8> {
    let first = &images[0];
    let mut output = Cursor::new(Vec::new());
    let track = TrackSpec {
        encoding: Encoding::Ffv1V1 {
            width: first.width as u32,
            height: first.height as u32,
        },
        name: "owned FFV1",
        language: "und",
    };
    let mut writer = PacketWriter::new(&mut output, &[track]).unwrap();
    for (i, frame) in images.iter().enumerate() {
        writer
            .write_packet(
                0,
                i as u64 * 40_000_000,
                40_000_000,
                true,
                &ffv1_encoder::encode(frame, depth).unwrap(),
            )
            .unwrap();
    }
    writer.finish().unwrap();
    output.into_inner()
}
#[test]
fn rejects_invalid_storage_and_encodes_independent_packets() {
    let first = image(17, 13, 2, 2, 10, 3);
    let packet = ffv1_encoder::encode(&first, 10).unwrap();
    assert!(!packet.is_empty());
    let _other = ffv1_encoder::encode(&image(8, 8, 1, 1, 16, 1), 16).unwrap();
    assert_eq!(packet, ffv1_encoder::encode(&first, 10).unwrap());
    let mut bad = image(2, 2, 2, 2, 10, 0);
    bad.data[1] = 4;
    assert!(ffv1_encoder::encode(&bad, 10).is_err());
    bad.data.pop();
    assert!(ffv1_encoder::encode(&bad, 10).is_err());
    bad.subsampling = None;
    assert!(ffv1_encoder::encode(&bad, 8).is_err());
    bad.subsampling = Some([0, 2]);
    assert!(ffv1_encoder::encode(&bad, 8).is_err());
    assert!(ffv1_encoder::encode(&first, 7).is_err());
    let flat = image(64, 64, 2, 2, 8, 0);
    assert!(ffv1_encoder::encode(&flat, 8).unwrap().len() < flat.data.len() / 10);
    assert!(!mux(&[first], 10).is_empty());
}
#[test]
fn owned_decoder_roundtrips_all_depths_and_subsampling() {
    use fvid_media::owned_ffv1_decoder::Decoder;
    for depth in 8..=16 {
        for (sx, sy) in [(1, 1), (2, 1), (2, 2), (1, 2), (4, 1), (4, 4)] {
            for (w, h) in [(1, 1), (8, 6), (17, 13)] {
                let mut decoder = Decoder::new(w, h, 1 << 20).unwrap();
                for pattern in 0..5 {
                    let original = image(w, h, sx, sy, depth, pattern);
                    let encoded = ffv1_encoder::encode(&original, depth).unwrap();
                    let decoded = decoder.decode(&encoded).unwrap();
                    assert!(decoded.keyframe);
                    assert_eq!(decoded.depth, depth);
                    assert_eq!(decoded.frame.subsampling, original.subsampling);
                    assert_eq!(
                        decoded.frame.data, original.data,
                        "{w}x{h} {depth} {sx}:{sy} pattern {pattern}"
                    );
                }
            }
        }
    }
}

#[test]
fn decoder_enforces_storage_and_recovers_at_keyframe() {
    use fvid_media::owned_ffv1_decoder::Decoder;
    let source = image(17, 13, 2, 2, 10, 3);
    let packet = ffv1_encoder::encode(&source, 10).unwrap();
    let mut decoder = Decoder::new(17, 13, 1 << 20).unwrap();
    assert!(decoder.decode(&[]).is_err());
    assert!(decoder.decode(&[255, 255]).is_err());
    assert!(decoder.decode(&packet[..packet.len() / 2]).is_err());
    assert_eq!(decoder.decode(&packet).unwrap().frame.data, source.data);
    decoder.reset();
    assert_eq!(decoder.decode(&packet).unwrap().frame.data, source.data);
    assert!(Decoder::new(17, 13, 100).unwrap().decode(&packet).is_err());
    assert!(Decoder::new(usize::MAX, 2, usize::MAX).is_err());
}

#[test]
fn malformed_packets_never_panic_or_poison_keyframe_recovery() {
    use fvid_media::owned_ffv1_decoder::Decoder;
    let original = image(8, 6, 2, 2, 10, 4);
    let packet = ffv1_encoder::encode(&original, 10).unwrap();
    let mut decoder = Decoder::new(8, 6, 1 << 20).unwrap();
    for at in 0..packet.len() {
        for value in [0, 1, 127, 255] {
            let mut corrupt = packet.clone();
            corrupt[at] = value;
            let _ = decoder.decode(&corrupt);
            assert_eq!(decoder.decode(&packet).unwrap().frame.data, original.data);
        }
    }
    // Arbitrary headers exercise invalid exponents, runs and model transitions.
    let mut random = 0x51384b762dc19u64;
    for len in 0..256 {
        let data: Vec<u8> = (0..len)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                random as u8
            })
            .collect();
        let _ = decoder.decode(&data);
    }
    assert_eq!(decoder.decode(&packet).unwrap().frame.data, original.data);
}

#[test]
fn library_streams_y4m_to_ffv1_with_exact_samples_and_rational_timing() {
    use fvid_media::owned_ffv1_decoder::Decoder;
    for (layout, sx, sy) in [
        ("420", 2, 2),
        ("422", 2, 1),
        ("444", 1, 1),
        ("440", 1, 2),
        ("411", 4, 1),
        ("410", 4, 4),
    ] {
        for depth in [8, 10, 16] {
            if sx == 4 && depth != 8 {
                continue;
            }
            let chroma = if depth == 8 {
                layout.into()
            } else {
                format!("{layout}p{depth}")
            };
            let mut input = format!("YUV4MPEG2 W4 H4 F3:2 Ip C{chroma}\n").into_bytes();
            let frames: Vec<_> = (0..3).map(|p| image(4, 4, sx, sy, depth, p)).collect();
            for frame in &frames {
                input.extend_from_slice(b"FRAME\n");
                input.extend_from_slice(&frame.data);
            }
            let transform = fvid_media::DecodeTransform {
                interval: Some((600_000, 1_400_000)),
                ..Default::default()
            };
            let mut decoder = Decoder::new(4, 4, 1 << 20).unwrap();
            let mut seen = 0;
            let stats = ffv1_encoder::encode_y4m(
                Cursor::new(input),
                &transform,
                |header, packet, pts, duration| {
                    assert_eq!((header.width, header.height), (4, 4));
                    let index = seen + 1;
                    let decoded = decoder.decode(packet).unwrap();
                    assert_eq!(decoded.frame.data, frames[index].data);
                    assert_eq!(decoded.frame.subsampling, Some([sx, sy]));
                    assert_eq!(decoded.depth, depth);
                    assert_eq!(pts, (index as u64 * 2_000_000_000) / 3);
                    assert_eq!(duration, ((index as u64 + 1) * 2_000_000_000) / 3 - pts);
                    seen += 1;
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(seen, 2);
            assert_eq!(stats.video_frames, 2);
        }
    }
}

#[test]
fn library_ffv1_stream_transposes_filters_and_stops_on_sink_error() {
    use fvid_media::owned_ffv1_decoder::Decoder;
    let source = image(4, 2, 2, 1, 10, 2);
    let mut input = b"YUV4MPEG2 W4 H2 F25:1 Ip C422p10\nFRAME\n".to_vec();
    input.extend_from_slice(&source.data);
    let transform = fvid_media::DecodeTransform {
        transpose: Some(fvid_media::TransposeMode::Clock),
        negate: Some("".into()),
        ..Default::default()
    };
    let mut expected = Vec::new();
    let mut offset = 0;
    for (w, h) in [(4, 2), (2, 2), (2, 2)] {
        for row in 0..w {
            for col in 0..h {
                let at = offset + ((h - 1 - col) * w + row) * 2;
                let sample = u16::from_le_bytes(source.data[at..at + 2].try_into().unwrap());
                expected.extend_from_slice(&(1023 - sample).to_le_bytes());
            }
        }
        offset += w * h * 2;
    }
    let mut decoder = Decoder::new(2, 4, 1 << 20).unwrap();
    let stats = ffv1_encoder::encode_y4m(
        Cursor::new(&input),
        &transform,
        |header, packet, pts, duration| {
            assert_eq!((header.width, header.height), (2, 4));
            let frame = decoder.decode(packet).unwrap();
            assert_eq!(frame.frame.subsampling, Some([1, 2]));
            assert_eq!(frame.frame.data, expected);
            assert_eq!((pts, duration), (0, 40_000_000));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!((stats.width, stats.height), (2, 4));
    let error = ffv1_encoder::encode_y4m(Cursor::new(&input), &transform, |_, _, _, _| {
        Err("sink stopped".into())
    })
    .unwrap_err();
    assert_eq!(error, "sink stopped");
    input.pop();
    let mut called = false;
    let error = ffv1_encoder::encode_y4m(Cursor::new(input), &transform, |_, _, _, _| {
        called = true;
        Ok(())
    })
    .unwrap_err();
    assert!(error.contains("truncated Y4M"));
    assert!(!called);
}

#[test]
fn library_matroska_writer_matches_frontend_and_exports_filtered_y4m() {
    use fvid_media::owned_ffv1_decoder::Decoder;
    use fvid::container::webm::WebmReader;
    let frame = image(4, 2, 2, 1, 10, 2);
    let packet = ffv1_encoder::encode(&frame, 10).unwrap();
    let mut original = Cursor::new(Vec::new());
    let mut expected = PacketWriter::new(
        &mut original,
        &[TrackSpec {
            encoding: Encoding::Ffv1V1 {
                width: 4,
                height: 2,
            },
            name: "",
            language: "und",
        }],
    )
    .unwrap();
    expected
        .write_packet(0, 666_666_666, 666_666_667, true, &packet)
        .unwrap();
    let expected_event = expected.finish().unwrap();
    let mut output = Cursor::new(Vec::new());
    let mut writer = fvid_media::owned_matroska::PacketWriter::new_ffv1(&mut output, 4, 2).unwrap();
    writer
        .write_packet(0, 666_666_666, 666_666_667, true, &packet)
        .unwrap();
    let event = writer.finish().unwrap();
    assert_eq!(
        (event.packets, event.payload_bytes, event.done),
        (
            expected_event.packets,
            expected_event.payload_bytes,
            expected_event.done
        )
    );
    assert_eq!(output.into_inner(), original.into_inner());

    let mut source = b"YUV4MPEG2 W4 H2 F3:2 Ip C422p10\n".to_vec();
    for _ in 0..3 {
        source.extend_from_slice(b"FRAME\n");
        source.extend_from_slice(&frame.data);
    }
    let transform = fvid_media::DecodeTransform {
        interval: Some((600_000, 1_400_000)),
        transpose: Some(fvid_media::TransposeMode::Clock),
        ..Default::default()
    };
    let mut output = Cursor::new(Vec::new());
    let (stats, event) =
        fvid_media::owned_matroska::write_y4m_ffv1(Cursor::new(source), &mut output, &transform)
            .unwrap();
    assert_eq!((stats.width, stats.height, stats.video_frames), (2, 4, 2));
    assert_eq!(event.packets, 2);
    assert!(!event.done);
    let mut reader =
        WebmReader::open(Cursor::new(output.into_inner()), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks[0].codec, "V_FFV1");
    assert_eq!(reader.packets.len(), 2);
    let mut decoder = Decoder::new(2, 4, 1 << 20).unwrap();
    let mut expected = Vec::new();
    let mut offset = 0;
    for (w, h) in [(4, 2), (2, 2), (2, 2)] {
        for row in 0..w {
            for col in 0..h {
                let at = offset + ((h - 1 - col) * w + row) * 2;
                expected.extend_from_slice(&frame.data[at..at + 2]);
            }
        }
        offset += w * h * 2;
    }
    for (i, block) in reader.packets.clone().iter().enumerate() {
        let index = i as u64 + 1;
        assert_eq!(block.pts_ns, (index * 2_000_000_000 / 3) as i64);
        let payload = reader.read_packet(i).unwrap();
        let decoded = decoder.decode(&payload).unwrap();
        assert_eq!(decoded.frame.data, expected);
        assert_eq!(decoded.frame.subsampling, Some([1, 2]));
    }
}

#[test]
fn library_matroska_rejects_partial_streams_and_keeps_failure_sticky() {
    use fvid_media::owned_matroska::{PacketWriter, write_y4m_ffv1};
    let mut output = Cursor::new(Vec::new());
    assert!(PacketWriter::new_ffv1(&mut output, 0, 4).is_err());
    assert!(output.get_ref().is_empty());
    let mut writer = PacketWriter::new_ffv1(&mut output, 4, 4).unwrap();
    assert!(writer.write_packet(1, 0, 1, true, &[1]).is_err());
    assert!(writer.write_packet(0, 0, 1, true, &[1]).is_err());
    assert!(writer.finish().is_err());
    let mut output = Cursor::new(Vec::new());
    assert!(
        write_y4m_ffv1(
            Cursor::new(b"YUV4MPEG2 W4 H4 F25:1 Ip C420\n"),
            &mut output,
            &Default::default()
        )
        .is_err()
    );
    assert!(output.get_ref().is_empty());
    let mut output = Cursor::new(Vec::new());
    assert!(
        write_y4m_ffv1(
            Cursor::new(b"YUV4MPEG2 W4 H4 F25:1 Ip C420\nFRAME\n\0"),
            &mut output,
            &Default::default()
        )
        .is_err()
    );
    assert!(output.get_ref().is_empty());
}

#[test]
fn library_file_export_publishes_after_sync_and_cleans_cancel_errors_and_races() {
    use fvid_control::{CancelFlag, ProgressHook};
    use fvid_media::owned_matroska::export_y4m_ffv1;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("fvid-library-mkv-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-vertical-chroma-10.y4m");
    let destination = directory.0.join("output.mkv");
    let published = destination.clone();
    let progress_count = Arc::new(AtomicUsize::new(0));
    let count = progress_count.clone();
    let hook = ProgressHook::new(move |event| {
        assert_eq!(published.exists(), event.done);
        assert!(event.packets > 0);
        count.fetch_add(1, Ordering::Relaxed);
    });
    let (stats, event) = export_y4m_ffv1(
        &source,
        &destination,
        &Default::default(),
        None,
        Some(&hook),
    )
    .unwrap();
    assert_eq!((stats.width, stats.height, stats.video_frames), (4, 2, 2));
    assert_eq!(event.packets, 2);
    assert!(event.done);
    assert_eq!(progress_count.load(Ordering::Relaxed), 3);
    let bytes = std::fs::read(&destination).unwrap();
    let mut reader =
        fvid::container::webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    reader.scan_all().unwrap();
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 2, 1 << 20).unwrap();
    for frame in 0..2 {
        let packet = reader.read_packet(frame).unwrap();
        let decoded = decoder.decode(&packet).unwrap();
        let expected: Vec<_> = (0..16)
            .flat_map(|i| (i * 17u16 + frame as u16 * 31).to_le_bytes())
            .collect();
        assert_eq!(decoded.frame.data, expected);
        assert_eq!(decoded.frame.subsampling, Some([1, 2]));
        assert_eq!(decoded.depth, 10);
    }
    assert!(export_y4m_ffv1(&source, &destination, &Default::default(), None, None).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), bytes);
    for before in [true, false] {
        let output = directory.0.join(format!("cancel-{before}.mkv"));
        let cancel = CancelFlag::new();
        if before {
            cancel.cancel();
        }
        let stop = cancel.clone();
        let hook = ProgressHook::new(move |event| {
            assert!(!event.done);
            stop.cancel();
        });
        let error = export_y4m_ffv1(
            &source,
            &output,
            &Default::default(),
            Some(&cancel),
            Some(&hook),
        )
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert!(!output.exists());
    }
    let race = directory.0.join("race.mkv");
    let other = race.clone();
    let hook = ProgressHook::new(move |event| {
        assert!(!event.done);
        if event.packets == 1 {
            std::fs::write(&other, b"other publisher").unwrap();
        }
    });
    assert!(export_y4m_ffv1(&source, &race, &Default::default(), None, Some(&hook)).is_err());
    assert_eq!(std::fs::read(&race).unwrap(), b"other publisher");
    let damaged = directory.0.join("truncated.y4m");
    let mut input = std::fs::read(&source).unwrap();
    input.pop();
    std::fs::write(&damaged, input).unwrap();
    let output = directory.0.join("truncated.mkv");
    assert!(
        export_y4m_ffv1(&damaged, &output, &Default::default(), None, None)
            .unwrap_err()
            .to_string()
            .contains("truncated")
    );
    assert!(!output.exists());
    assert!(std::fs::read_dir(&directory.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-matroska-")
    }));
}

#[test]
fn y4m_metadata_survives_export_and_geometry_without_float_rounding() {
    use fvid_media::{CropRect, DecodeTransform, PadRect, ScaleSize, TransposeMode};
    let input = include_bytes!("fixtures/playback-errors/y4m-aspect-full-10.y4m");
    for (transform, ratio) in [
        (DecodeTransform::default(), (16, 15)),
        (
            DecodeTransform {
                transpose: Some(TransposeMode::Clock),
                ..Default::default()
            },
            (15, 16),
        ),
        (
            DecodeTransform {
                scale: Some(ScaleSize {
                    width: 8,
                    height: 2,
                }),
                ..Default::default()
            },
            (8, 15),
        ),
        (
            DecodeTransform {
                transpose: Some(TransposeMode::Clock),
                scale: Some(ScaleSize {
                    width: 4,
                    height: 4,
                }),
                ..Default::default()
            },
            (15, 32),
        ),
        (
            DecodeTransform {
                crop: Some(CropRect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                }),
                ..Default::default()
            },
            (16, 15),
        ),
        (
            DecodeTransform {
                pad: Some(PadRect {
                    width: 6,
                    height: 4,
                    x: 0,
                    y: 0,
                }),
                ..Default::default()
            },
            (16, 15),
        ),
        (
            DecodeTransform {
                pad: Some(PadRect {
                    width: 6,
                    height: 4,
                    x: 0,
                    y: 0,
                }),
                scale: Some(ScaleSize {
                    width: 4,
                    height: 4,
                }),
                ..Default::default()
            },
            (8, 5),
        ),
    ] {
        let mut output = Cursor::new(Vec::new());
        let (stats, event) =
            fvid_media::owned_matroska::write_y4m_ffv1(Cursor::new(input), &mut output, &transform)
                .unwrap();
        assert_eq!(event.packets, 2);
        let mut reader = fvid::container::webm::WebmReader::open(
            Cursor::new(output.into_inner()),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks[0].pixel_aspect(), ratio);
        assert!(reader.tracks[0].colour.full_range);
        assert_eq!(reader.tracks[0].colour.matrix, 6);
        if transform == DecodeTransform::default() {
            let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(
                stats.width as usize,
                stats.height as usize,
                1 << 20,
            )
            .unwrap();
            for frame in 0..2 {
                let packet = reader.read_packet(frame).unwrap();
                let expected: Vec<_> = (0..16)
                    .flat_map(|i| (i * 17u16 + frame as u16 * 31).to_le_bytes())
                    .collect();
                assert_eq!(decoder.decode(&packet).unwrap().frame.data, expected);
            }
        }
    }
    for bad in [
        "A16:0 XCOLORRANGE=FULL",
        "A16:15 A1:1 XCOLORRANGE=FULL",
        "A16:15 XCOLORRANGE=INVALID",
        "A16:15 XCOLORRANGE=FULL XCOLORRANGE=LIMITED",
    ] {
        let mut source = format!("YUV4MPEG2 W4 H2 F30:1 Ip C440p10 {bad}\nFRAME\n").into_bytes();
        source.extend_from_slice(&[0; 32]);
        let mut output = Cursor::new(Vec::new());
        assert!(
            fvid_media::owned_matroska::write_y4m_ffv1(
                Cursor::new(source),
                &mut output,
                &Default::default()
            )
            .is_err()
        );
        assert!(output.get_ref().is_empty());
    }
}

#[test]
fn public_lossless_api_uses_owned_y4m_export_and_enforces_packet_policy() {
    let directory =
        std::env::temp_dir().join(format!("fvid-owned-lossless-api-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-aspect-full-10.y4m");
    let transform = fvid_media::LosslessTransform {
        transpose: Some(fvid_media::TransposeMode::Clock),
        negate: Some("".into()),
        ..Default::default()
    };
    let output = directory.join("api.mkv");
    let stats =
        fvid_media::transcode_lossless(&source, &output, transform.clone(), &Default::default())
            .unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.video_frames, 2);
    assert_eq!(stats.pixel_format, "yuv422p10le");
    let mut expected = Cursor::new(Vec::new());
    fvid_media::owned_matroska::write_y4m_ffv1(
        Cursor::new(std::fs::read(&source).unwrap()),
        &mut expected,
        &fvid_media::DecodeTransform {
            transpose: transform.transpose,
            negate: transform.negate.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), expected.into_inner());
    let explicit = directory.join("explicit.mkv");
    let settings = fvid_media::EncoderSettings {
        name: "ffv1".into(),
        options: vec![],
    };
    let explicit_stats = fvid_media::transcode(
        &source,
        &explicit,
        transform.clone(),
        &Default::default(),
        &settings,
    )
    .unwrap();
    assert_eq!(explicit_stats.backend, "fvid");
    assert_eq!(
        std::fs::read(&explicit).unwrap(),
        std::fs::read(&output).unwrap()
    );
    let unsupported = fvid_media::EncoderSettings {
        name: "ffv1".into(),
        options: vec![("level".into(), "3".into())],
    };
    let refused = directory.join("unsupported.mkv");
    assert!(
        fvid_media::owned_lossless::transcode(
            &source,
            &refused,
            transform.clone(),
            &Default::default(),
            &unsupported
        )
        .unwrap_err()
        .contains("encoder/settings")
    );
    assert!(!refused.exists());
    let limited = directory.join("limited.mkv");
    let options = fvid_media::CopyOptions {
        max_packet_bytes: 1,
        ..Default::default()
    };
    assert!(
        fvid_media::transcode_lossless(&source, &limited, transform, &options)
            .unwrap_err()
            .contains("byte limit")
    );
    assert!(!limited.exists());
    assert!(std::fs::read_dir(&directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-matroska-")
    }));
    let stats = fvid_media::owned_lossless::transcode_lossless(
        &source, &limited,
        fvid_media::LosslessTransform { hue: Some("h=90".into()), ..Default::default() },
        &Default::default(),
    ).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert!(stats.video_frames > 0);

}

#[test]
fn lossless_packet_count_limit_rejects_before_reading_extra_frame_payload() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let directory =
        std::env::temp_dir().join(format!("fvid-owned-packet-cap-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-aspect-full-10.y4m");
    for limit in 0..4 {
        let destination = directory.join(format!("limit-{limit}.mkv"));
        let packets = Arc::new(AtomicU64::new(0));
        let count = packets.clone();
        let published = destination.clone();
        let options = fvid_media::CopyOptions {
            max_packets: Some(limit),
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                assert_eq!(published.exists(), event.done);
                count.store(event.packets, Ordering::Relaxed);
            })),
            ..Default::default()
        };
        let result =
            fvid_media::transcode_lossless(&source, &destination, Default::default(), &options);
        if limit < 2 {
            assert!(result.unwrap_err().contains("packet count exceeds limit"));
            assert!(!destination.exists());
        } else {
            assert_eq!(result.unwrap().video_frames, 2);
            assert!(destination.exists());
        }
        assert_eq!(packets.load(Ordering::Relaxed), limit.min(2));
    }
    let truncated = directory.join("truncated.y4m");
    let mut input = std::fs::read(&source).unwrap();
    input.pop();
    std::fs::write(&truncated, input).unwrap();
    let destination = directory.join("truncated.mkv");
    let options = fvid_media::CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    let error =
        fvid_media::transcode_lossless(&truncated, &destination, Default::default(), &options)
            .unwrap_err();
    assert!(error.contains("packet count exceeds limit"), "{error}");
    assert!(!destination.exists());
    assert!(std::fs::read_dir(&directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-matroska-")
    }));
}

#[test]
fn public_crop_lossless_preserves_exact_high_depth_planes_and_aspect() {
    let directory =
        std::env::temp_dir().join(format!("fvid-owned-crop-api-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-aspect-full-10.y4m");
    let destination = directory.join("crop.mkv");
    let stats = fvid_media::crop_lossless(
        &source,
        &destination,
        fvid_media::CropRect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        },
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.video_frames, 2);
    let mut reader = fvid::container::webm::WebmReader::open(
        Cursor::new(std::fs::read(&destination).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks[0].pixel_aspect(), (16, 15));
    assert!(reader.tracks[0].colour.full_range);
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(2, 2, 1 << 20).unwrap();
    for frame in 0..2 {
        let packet = reader.read_packet(frame).unwrap();
        let expected: Vec<_> = [1, 2, 5, 6, 9, 10, 13, 14]
            .into_iter()
            .flat_map(|i| (i * 17u16 + frame as u16 * 31).to_le_bytes())
            .collect();
        let decoded = decoder.decode(&packet).unwrap();
        assert_eq!(decoded.frame.data, expected);
        assert_eq!(decoded.depth, 10);
        assert_eq!(decoded.frame.subsampling, Some([1, 2]));
    }
    let invalid = directory.join("invalid.mkv");
    assert!(
        fvid_media::crop_lossless(
            &source,
            &invalid,
            fvid_media::CropRect {
                x: 0,
                y: 1,
                width: 2,
                height: 1
            },
            &Default::default()
        )
        .is_err()
    );
    assert!(!invalid.exists());
}

#[test]
fn public_lossless_interval_rebases_clock_and_counts_preroll_packets() {
    let directory =
        std::env::temp_dir().join(format!("fvid-owned-interval-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-interval-25-10.y4m");
    let transform = fvid_media::LosslessTransform {
        interval: Some((40_000, 80_000)),
        ..Default::default()
    };
    let output = directory.join("interval.mkv");
    let stats = fvid_media::transcode_lossless(
        &source,
        &output,
        transform.clone(),
        &fvid_media::CopyOptions {
            max_packets: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.video_frames, 1);
    assert_eq!(stats.decoded_frames, 2);
    assert!(!stats.seek_used);
    let mut reader = fvid::container::webm::WebmReader::open(
        Cursor::new(std::fs::read(&output).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.packets.len(), 1);
    assert_eq!(reader.packets[0].pts_ns, 0);
    assert_eq!(reader.duration_ns, Some(40_000_000));
    let packet = reader.read_packet(0).unwrap();
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 2, 1 << 20).unwrap();
    let expected: Vec<_> = (0..16)
        .flat_map(|i| (i * 17u16 + 31).to_le_bytes())
        .collect();
    assert_eq!(decoder.decode(&packet).unwrap().frame.data, expected);
    let limited = directory.join("limited.mkv");
    assert!(
        fvid_media::transcode_lossless(
            &source,
            &limited,
            transform,
            &fvid_media::CopyOptions {
                max_packets: Some(1),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("packet count exceeds limit")
    );
    assert!(!limited.exists());
    for (i, interval) in [
        (1, 80_000),
        (-1, 80_000),
        (40_000, 40_000),
        (80_000, 120_000),
    ]
    .into_iter()
    .enumerate()
    {
        let output = directory.join(format!("invalid-{i}.mkv"));
        let error = fvid_media::transcode_lossless(
            &source,
            &output,
            fvid_media::LosslessTransform {
                interval: Some(interval),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap_err();
        if i == 0 {
            assert!(error.contains("not exact in video time base"));
        }
        if i == 3 {
            assert!(error.contains("no selected frames"));
        }
        assert!(!output.exists());
    }
    assert!(std::fs::read_dir(&directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-matroska-")
    }));
}

#[test]
fn public_lossless_metadata_edits_are_ordered_and_never_drop_unknown_keys() {
    let directory =
        std::env::temp_dir().join(format!("fvid-owned-tags-policy-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-aspect-full-10.y4m");
    let output = directory.join("tags.mkv");
    let options = fvid_media::CopyOptions {
        metadata_delete: vec!["title".into()],
        metadata_set: vec![
            ("TITLE".into(), "first".into()),
            ("title".into(), "Последний 🎬".into()),
            ("artist".into(), "removed".into()),
            ("ARTIST".into(), "".into()),
            ("TRACKNUMBER".into(), "3/12".into()),
            ("albumartist".into(), "Album artist".into()),
        ],
        ..Default::default()
    };
    let stats =
        fvid_media::transcode_lossless(&source, &output, Default::default(), &options).unwrap();
    assert_eq!(stats.backend, "fvid");
    let mut reader = fvid::container::webm::WebmReader::open(
        Cursor::new(std::fs::read(&output).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tags.title, "Последний 🎬");
    assert!(reader.tags.artist.is_empty());
    assert_eq!(reader.tags.track, "3/12");
    assert_eq!(reader.tags.album_artist, "Album artist");
    let custom = directory.join("custom.mkv");
    fvid_media::owned_lossless::transcode_lossless(&source, &custom, Default::default(),
        &fvid_media::CopyOptions { metadata_set: vec![("unknown".into(), "keep me".into())], ..Default::default() }).unwrap();
    let mut reader = fvid_media::owned_webm::WebmReader::open(Cursor::new(std::fs::read(custom).unwrap()), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.metadata["UNKNOWN"], "keep me");
    for (index, entries) in [
        vec![("title".into(), "bad\0value".into())],
        vec![("title".into(), "x".into()); 65],
    ]
    .into_iter()
    .enumerate()
    {
        let output = directory.join(format!("invalid-{index}.mkv"));
        let options = fvid_media::CopyOptions {
            metadata_set: entries,
            ..Default::default()
        };
        let error = fvid_media::owned_lossless::transcode_lossless(
            &source,
            &output,
            Default::default(),
            &options,
        )
        .unwrap_err();
        assert!(!output.exists());
        assert!(error.contains(if index == 0 { "NUL" } else { "at most 64" }));
    }
}
