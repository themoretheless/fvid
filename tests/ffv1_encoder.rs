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
#[ignore = "requires FVID_REFERENCE_FFMPEG; independent decoder only"]
fn packets_decode_losslessly_in_independent_decoder() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let ffmpeg = std::env::var("FVID_REFERENCE_FFMPEG").unwrap();
    for (depth, format, sx, sy) in [
        (8, "yuv420p", 2, 2),
        (8, "yuv422p", 2, 1),
        (8, "yuv444p", 1, 1),
        (9, "yuv420p9le", 2, 2),
        (10, "yuv420p10le", 2, 2),
        (12, "yuv422p12le", 2, 1),
        (14, "yuv444p14le", 1, 1),
        (16, "yuv420p16le", 2, 2),
        (16, "yuv444p16le", 1, 1),
    ] {
        for (w, h) in [(1, 1), (8, 6), (17, 13)] {
            let frames: Vec<_> = (0..5)
                .map(|pattern| image(w, h, sx, sy, depth, pattern))
                .collect();
            let encoded = mux(&frames, depth);
            let expected: Vec<_> = frames.iter().flat_map(|f| f.data.iter().copied()).collect();
            let mut p = Command::new(&ffmpeg)
                .args([
                    "-v", "error", "-i", "pipe:0", "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt",
                    format, "pipe:1",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            p.stdin.take().unwrap().write_all(&encoded).unwrap();
            let out = p.wait_with_output().unwrap();
            assert!(
                out.status.success(),
                "{w}x{h} {format}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                out.stderr.is_empty(),
                "{w}x{h} {format}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(out.stdout.len(), expected.len(), "{w}x{h} {format}");
            if let Some(i) = expected.iter().zip(&out.stdout).position(|(a, b)| a != b) {
                panic!(
                    "{w}x{h} {format} byte {i}: input={} decoded={}",
                    expected[i], out.stdout[i]
                );
            }
        }
    }
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG; independent decoder only"]
fn owned_avc_hevc_decode_to_ffv1_preserves_all_samples() {
    use fvid::playback_native::{NativeReader, RawFrame};
    use std::{
        fs::File,
        io::{BufReader, Write},
        process::{Command, Stdio},
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let ffmpeg = std::env::var("FVID_REFERENCE_FFMPEG").unwrap();
    for (name, format) in [
        ("video.mp4", "yuv420p"),
        ("hevc/main-ipb.mp4", "yuv420p"),
        ("hevc/main10-ipb.mp4", "yuv420p10le"),
    ] {
        let mut reader = NativeReader::software(
            BufReader::new(File::open(root.join("tests/fixtures").join(name)).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let mut frames = Vec::new();
        let mut depth = 8;
        while let Some(raw) = reader.read_frame_raw().unwrap() {
            if let RawFrame::Avc { picture, .. } = &raw {
                depth = picture.bit_depth;
            }
            let [w, h] = reader.dimensions();
            frames.push(
                fvid::native_geometry::VideoGeometry::default()
                    .apply_display(&raw, w, h, reader.rotation())
                    .unwrap(),
            );
        }
        assert!(!frames.is_empty());
        let encoded = mux(&frames, depth);
        let expected: Vec<_> = frames.iter().flat_map(|f| f.data.iter().copied()).collect();
        let mut child = Command::new(&ffmpeg)
            .args([
                "-v", "error", "-i", "pipe:0", "-f", "rawvideo", "-pix_fmt", format, "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || stdin.write_all(&encoded));
        let out = child.wait_with_output().unwrap();
        writer.join().unwrap().unwrap();
        assert!(
            out.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.stderr.is_empty(),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.stdout, expected, "{name}");
    }
}

#[test]
fn owned_decoder_roundtrips_all_depths_and_subsampling() {
    use fvid::codec::ffv1_decoder::Decoder;
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
    use fvid::codec::ffv1_decoder::Decoder;
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
#[ignore = "requires FVID_REFERENCE_FFMPEG; independent encoder only"]
fn owned_decoder_reads_independently_encoded_contexts_and_nonkeyframes() {
    use fvid::{
        codec::ffv1_decoder::Decoder,
        container::webm::{Limits, WebmReader},
    };
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let ffmpeg = std::env::var("FVID_REFERENCE_FFMPEG").unwrap();
    for (depth, format, sx, sy) in [
        (8, "yuv420p", 2, 2),
        (8, "yuv422p", 2, 1),
        (8, "yuv444p", 1, 1),
        (10, "yuv420p10le", 2, 2),
        (12, "yuv422p12le", 2, 1),
        (16, "yuv444p16le", 1, 1),
    ] {
        for level in if depth == 8 {
            vec!["0", "1"]
        } else {
            vec!["1"]
        } {
            for coder in ["-2", "2"] {
                for context in ["0", "1"] {
                    let (w, h) = (17, 13);
                    let frames: Vec<_> = (0..7).map(|i| image(w, h, sx, sy, depth, i)).collect();
                    let input: Vec<_> =
                        frames.iter().flat_map(|f| f.data.iter().copied()).collect();
                    let mut child = Command::new(&ffmpeg)
                        .args([
                            "-v",
                            "error",
                            "-f",
                            "rawvideo",
                            "-pixel_format",
                            format,
                            "-video_size",
                            "17x13",
                            "-framerate",
                            "25",
                            "-i",
                            "pipe:0",
                            "-c:v",
                            "ffv1",
                            "-level",
                            level,
                            "-coder",
                            coder,
                            "-context",
                            context,
                            "-g",
                            "3",
                            "-threads",
                            "1",
                            "-f",
                            "matroska",
                            "pipe:1",
                        ])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .unwrap();
                    child.stdin.take().unwrap().write_all(&input).unwrap();
                    let output = child.wait_with_output().unwrap();
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let mut reader =
                        WebmReader::open(Cursor::new(output.stdout.clone()), Limits::default())
                            .unwrap();
                    reader.scan_all().unwrap();
                    assert_eq!(reader.packets.len(), frames.len());
                    let mut decoder = Decoder::new(w, h, 8 << 20).unwrap();
                    let mut nonkeys = 0;
                    for (i, expected) in frames.iter().enumerate() {
                        let packet = reader.read_packet(i).unwrap();
                        let decoded = decoder.decode(&packet).unwrap_or_else(|e| {
                            panic!("{format} coder {coder} context {context} frame {i}: {e}")
                        });
                        nonkeys += usize::from(!decoded.keyframe);
                        assert_eq!(decoded.depth, depth);
                        assert_eq!(
                            decoded.frame.data, expected.data,
                            "{format} coder {coder} context {context} frame {i}"
                        );
                        if !decoded.keyframe {
                            assert!(
                                Decoder::new(w, h, 8 << 20)
                                    .unwrap()
                                    .decode(&packet)
                                    .is_err()
                            );
                        }
                    }
                    assert!(nonkeys > 0);
                    // Exercise the production container dispatch as well as the packet API.
                    use fvid::playback_native::{NativeReader, RawFrame};
                    let mut native =
                        NativeReader::software(Cursor::new(output.stdout), 8 << 20).unwrap();
                    for expected in &frames {
                        let RawFrame::Planar(p) = native.read_frame_raw().unwrap().unwrap() else {
                            panic!("expected full-precision planar frame")
                        };
                        assert_eq!(p.depth, depth);
                        assert_eq!(p.frame.data, expected.data);
                    }
                    assert!(native.read_frame_raw().unwrap().is_none());
                    for index in [4usize, 1, 6, 0] {
                        let RawFrame::Planar(p) = native
                            .seek_raw(std::time::Duration::from_millis(index as u64 * 40 + 1))
                            .unwrap()
                            .unwrap()
                        else {
                            panic!("expected full-precision seek")
                        };
                        assert_eq!(
                            p.frame.data, frames[index].data,
                            "{format} coder {coder} context {context} seek {index}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn malformed_packets_never_panic_or_poison_keyframe_recovery() {
    use fvid::codec::ffv1_decoder::Decoder;
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
    use fvid::codec::ffv1_decoder::Decoder;
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
    use fvid::codec::ffv1_decoder::Decoder;
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
    use fvid::codec::ffv1_decoder::Decoder;
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
    let mut decoder = fvid::codec::ffv1_decoder::Decoder::new(4, 2, 1 << 20).unwrap();
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
