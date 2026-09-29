use fvid::{
    codec::ffv1_encoder,
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
