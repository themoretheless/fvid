use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn directory() -> Directory {
    let path = std::env::temp_dir().join(format!(
        "fvid-export-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    Directory(path)
}
#[test]
fn exports_main10_exactly_and_never_overwrites() {
    let dir = directory();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4");
    let output = dir.0.join("out.y4m");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "export-y4m"])
        .arg(&source)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("\"video_frames\":17"));
    let bytes = std::fs::read(&output).unwrap();
    let end = bytes.iter().position(|b| *b == b'\n').unwrap() + 1;
    let header = std::str::from_utf8(&bytes[..end]).unwrap();
    assert!(header.contains("F30:1"), "{header}");
    assert!(header.contains("C420p10"), "{header}");
    let oracle = include_bytes!("fixtures/hevc/main10-ipb.yuv");
    let frame_size = oracle.len() / 17;
    let mut payload = &bytes[end..];
    for expected in oracle.chunks_exact(frame_size) {
        assert!(payload.starts_with(b"FRAME\n"));
        payload = &payload[6..];
        assert!(payload[..frame_size] == *expected);
        payload = &payload[frame_size..];
    }
    assert!(payload.is_empty());
    assert!(fvid::native_export::export_y4m(&source, &output).is_err());
    assert!(std::fs::read(&output).unwrap() == bytes);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}
#[test]
fn decode_failure_leaves_no_output_or_temporary_file() {
    let dir = directory();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    assert!(fvid::native_export::export_y4m(&source, &dir.0.join("bad.y4m")).is_err());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[test]
fn truncated_input_cleans_partial_export() {
    let dir = directory();
    let source = dir.0.join("truncated.y4m");
    let mut input = b"YUV4MPEG2 W2 H2 F30:1 Ip A1:1 C420\nFRAME\n".to_vec();
    input.extend_from_slice(&[16, 16, 16, 16, 128, 128]);
    input.extend_from_slice(b"FRAME\n");
    std::fs::write(&source, input).unwrap();
    let output = dir.0.join("out.y4m");
    assert!(fvid::native_export::export_y4m(&source, &output).is_err());
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn a_fragment_timestamp_gap_is_not_silently_retimed() {
    let dir = directory();
    let mut data = include_bytes!("fixtures/video.mp4").to_vec();
    let offsets: Vec<_> = data
        .windows(4)
        .enumerate()
        .filter_map(|(i, bytes)| (bytes == b"tfdt").then_some(i))
        .collect();
    assert_eq!(
        offsets.len(),
        5,
        "fixture must contain five timed fragments"
    );
    let at = offsets[1];
    match data[at + 4] {
        0 => {
            let value = u32::from_be_bytes(data[at + 8..at + 12].try_into().unwrap());
            data[at + 8..at + 12].copy_from_slice(&(value + 512).to_be_bytes());
        }
        1 => {
            let value = u64::from_be_bytes(data[at + 8..at + 16].try_into().unwrap());
            data[at + 8..at + 16].copy_from_slice(&(value + 512).to_be_bytes());
        }
        _ => panic!("unexpected tfdt version"),
    }
    let input = dir.0.join("gap.mp4");
    std::fs::write(&input, data).unwrap();
    let output = dir.0.join("out.y4m");
    let error = fvid::native_export::export_y4m(&input, &output).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("constant contiguous frame timing"),
        "{error}"
    );
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn y4m_roundtrip_preserves_non_square_pixels_and_samples() {
    let dir = directory();
    let input = dir.0.join("input.y4m");
    let output = dir.0.join("output.y4m");
    let mut bytes = b"YUV4MPEG2 W2 H2 F30000:1001 Ip A2:1 C420\nFRAME\n".to_vec();
    bytes.extend_from_slice(&[16, 235, 80, 160, 90, 180]);
    std::fs::write(&input, &bytes).unwrap();
    assert_eq!(fvid::native_export::export_y4m(&input, &output).unwrap(), 1);
    let encoded = std::fs::read(&output).unwrap();
    let header =
        std::str::from_utf8(&encoded[..encoded.iter().position(|v| *v == b'\n').unwrap()]).unwrap();
    assert!(header.contains("A2:1"), "{header}");
    assert!(header.contains("F30000:1001"), "{header}");
    assert!(encoded.ends_with(&bytes[bytes.len() - 12..]));
    let reader =
        fvid::playback_native::NativeReader::software(std::io::Cursor::new(encoded), usize::MAX)
            .unwrap();
    assert_eq!(reader.pixel_aspect(), (2, 1));
}

#[test]
fn y4m_pixel_aspect_rejects_ambiguous_or_invalid_ratios() {
    for aspect in ["A0:1", "A1:0", "A-1:1", "A2:1 A3:1", "A4294967296:1"] {
        let bytes = format!("YUV4MPEG2 W2 H2 F30:1 Ip {aspect} C420\n");
        assert!(
            fvid::playback_native::NativeReader::software(std::io::Cursor::new(bytes), 1024)
                .is_err(),
            "{aspect}"
        );
    }
    let bytes = b"YUV4MPEG2 W2 H2 F30:1 Ip A0:0 C420\n";
    let reader =
        fvid::playback_native::NativeReader::software(std::io::Cursor::new(bytes), 1024).unwrap();
    assert_eq!(reader.pixel_aspect(), (1, 1));
}

#[test]
fn full_range_y4m_roundtrip_keeps_black_white_and_range_tag() {
    let dir = directory();
    let input = dir.0.join("full.y4m");
    let output = dir.0.join("out.y4m");
    let mut bytes = b"YUV4MPEG2 W2 H2 F30:1 Ip C420 XCOLORRANGE=FULL\nFRAME\n".to_vec();
    bytes.extend_from_slice(&[0, 255, 16, 235, 128, 128]);
    std::fs::write(&input, &bytes).unwrap();
    let mut reader =
        fvid::playback_native::NativeReader::software(std::io::Cursor::new(bytes), 1024).unwrap();
    assert!(reader.read_frame().unwrap());
    assert_eq!(
        reader.rgb(),
        &[0, 0, 0, 255, 255, 255, 16, 16, 16, 235, 235, 235]
    );
    reader.rewind().unwrap();
    let raw = reader.read_frame_raw().unwrap().unwrap();
    assert_eq!(raw.into_rgb(1024).unwrap(), reader.rgb());
    assert_eq!(fvid::native_export::export_y4m(&input, &output).unwrap(), 1);
    let bytes = std::fs::read(output).unwrap();
    assert!(
        bytes
            .windows(b"XCOLORRANGE=FULL\n".len())
            .any(|w| w == b"XCOLORRANGE=FULL\n")
    );
    assert!(bytes.ends_with(&[0, 255, 16, 235, 128, 128]));
}

#[test]
fn full_range_colour_agrees_between_direct_and_raw_readers() {
    let mut bytes = b"YUV4MPEG2 W2 H2 F30:1 C420 XCOLORRANGE=FULL\nFRAME\n".to_vec();
    bytes.extend_from_slice(&[0, 255, 100, 180, 32, 220]);
    let mut reader =
        fvid::playback_native::NativeReader::software(std::io::Cursor::new(bytes), 1024).unwrap();
    reader.read_frame().unwrap();
    let expected = reader.rgb().to_vec();
    reader.rewind().unwrap();
    let actual = reader
        .read_frame_raw()
        .unwrap()
        .unwrap()
        .into_rgb(1024)
        .unwrap();
    assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1));
    for range in [
        "XCOLORRANGE=INVALID",
        "XCOLORRANGE=FULL XCOLORRANGE=LIMITED",
    ] {
        let header = format!("YUV4MPEG2 W2 H2 F30:1 C420 {range}\n");
        assert!(
            fvid::playback_native::NativeReader::software(std::io::Cursor::new(header), 1024)
                .is_err()
        );
    }
}

#[test]
fn interval_export_preserves_main10_pixels_and_half_open_boundaries() {
    let dir = directory();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4");
    let output = dir.0.join("interval.y4m");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "export-y4m"])
        .arg(&source).arg(&output)
        .args(["--from", "0.1", "--to", "0.2"])
        .output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(String::from_utf8_lossy(&run.stdout).contains("\"video_frames\":3"));
    let bytes = std::fs::read(&output).unwrap();
    let end = bytes.iter().position(|b| *b == b'\n').unwrap() + 1;
    assert!(std::str::from_utf8(&bytes[..end]).unwrap().contains("C420p10"));
    let oracle = include_bytes!("fixtures/hevc/main10-ipb.yuv");
    let size = oracle.len() / 17;
    let expected: Vec<u8> = oracle.chunks_exact(size).skip(3).take(3)
        .flat_map(|frame| b"FRAME\n".iter().chain(frame).copied()).collect();
    assert_eq!(&bytes[end..], expected);
}

#[test]
fn empty_or_invalid_interval_does_not_publish_output() {
    use std::time::Duration;
    let dir = directory();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4");
    let output = dir.0.join("empty.y4m");
    for (from, to) in [(2, 3), (1, 1), (2, 1)] {
        assert!(fvid::native_export::export_y4m_interval(&source, &output,
            Some((Duration::from_secs(from), Duration::from_secs(to)))).is_err());
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
    }
}

#[test]
fn rotated_main10_export_turns_every_sample_without_precision_loss() {
    let dir = directory();
    let source = dir.0.join("rotated.mp4");
    let fixture = include_bytes!("fixtures/hevc/main10-ipb.mp4");
    let oracle = include_bytes!("fixtures/hevc/main10-ipb.yuv");
    // The saved oracle is 128x128 planar 4:2:0, 16-bit sample storage.
    let size = oracle.len() / 17;
    assert_eq!(size, 128 * 128 * 3);
    for rotation in [90, 180, 270] {
        let mut mp4 = fixture.to_vec();
        let at = mp4.windows(4).position(|w| w == b"tkhd").unwrap();
        assert_eq!(mp4[at + 4], 0);
        let matrix = at + 44;
        let values: [i32; 4] = match rotation {
            90 => [0, 65536, -65536, 0],
            180 => [-65536, 0, 0, -65536],
            _ => [0, -65536, 65536, 0],
        };
        for (offset, value) in [0, 4, 12, 16].into_iter().zip(values) {
            mp4[matrix + offset..matrix + offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        std::fs::write(&source, mp4).unwrap();
        let output = dir.0.join(format!("{rotation}.y4m"));
        assert_eq!(fvid::native_export::export_y4m(&source, &output).unwrap(), 17);
        let data = std::fs::read(output).unwrap();
        let end = data.iter().position(|b| *b == b'\n').unwrap() + 1;
        let header = std::str::from_utf8(&data[..end]).unwrap();
        assert!(header.contains("W128 H128"));
        let mut expected = Vec::new();
        for frame in oracle.chunks_exact(size) {
            expected.extend_from_slice(b"FRAME\n");
            let mut offset = 0;
            for (w, h) in [(128, 128), (64, 64), (64, 64)] {
                let plane = &frame[offset..offset + w * h * 2];
                let (ow, oh) = if rotation == 180 { (w, h) } else { (h, w) };
                let mut turned = vec![0; plane.len()];
                // Independent forward mapping from source to destination.
                for y in 0..h { for x in 0..w {
                    let (dx, dy) = match rotation {
                        90 => (h - 1 - y, x),
                        180 => (w - 1 - x, h - 1 - y),
                        _ => (y, w - 1 - x),
                    };
                    assert!(dy < oh);
                    turned[(dy * ow + dx) * 2..(dy * ow + dx) * 2 + 2]
                        .copy_from_slice(&plane[(y * w + x) * 2..(y * w + x) * 2 + 2]);
                }}
                expected.extend(turned);
                offset += w * h * 2;
            }
        }
        assert_eq!(&data[end..], expected);
    }
}

#[test]
fn quarter_turn_inverts_exported_pixel_aspect() {
    let dir = directory();
    let mut mp4 = include_bytes!("fixtures/display/par-2x1.mp4").to_vec();
    let matrix = mp4.windows(4).position(|w| w == b"tkhd").unwrap() + 44;
    for (offset, value) in [(0, 0i32), (4, 65536), (12, -65536), (16, 0)] {
        mp4[matrix + offset..matrix + offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    let source = dir.0.join("par.mp4");
    let output = dir.0.join("par.y4m");
    std::fs::write(&source, mp4).unwrap();
    fvid::native_export::export_y4m(&source, &output).unwrap();
    let bytes = std::fs::read(output).unwrap();
    let end = bytes.iter().position(|b| *b == b'\n').unwrap();
    assert!(std::str::from_utf8(&bytes[..end]).unwrap().contains("A1:2"));
}

#[test]
fn transformed_main10_export_keeps_reference_pixels_and_black_padding() {
    let dir = directory();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4");
    let output = dir.0.join("geometry.y4m");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "export-y4m"])
        .arg(&source)
        .arg(&output)
        .args([
            "--crop",
            "2:4:32:16",
            "--hflip",
            "--transpose",
            "clock",
            "--pad",
            "20:36:2:2",
            "--from",
            "0.1",
            "--to",
            "0.2",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let bytes = std::fs::read(&output).unwrap();
    let end = bytes.iter().position(|&b| b == b'\n').unwrap() + 1;
    let header = std::str::from_utf8(&bytes[..end]).unwrap();
    assert!(header.contains("W20 H36 F30:1 Ip A1:1 C420p10"), "{header}");
    let reference = include_bytes!("fixtures/hevc/main10-ipb.yuv");
    let mut expected = Vec::new();
    for frame in reference.chunks_exact(128 * 128 * 3).skip(3).take(3) {
        expected.extend_from_slice(b"FRAME\n");
        let mut source_offset = 0;
        for (sw, sh, cx, cy, cw, ch, ow, oh, pad, black) in [
            (128, 128, 2, 4, 32, 16, 20, 36, 2, 64u16),
            (64, 64, 1, 2, 16, 8, 10, 18, 1, 512),
            (64, 64, 1, 2, 16, 8, 10, 18, 1, 512),
        ] {
            let mut plane = black.to_le_bytes().repeat(ow * oh);
            // Independent forward mapping: reflect the cropped x, then rotate.
            for y in 0..ch {
                for x in 0..cw {
                    let at = source_offset + ((cy + y) * sw + cx + x) * 2;
                    let (dx, dy) = (pad + ch - 1 - y, pad + cw - 1 - x);
                    plane[(dy * ow + dx) * 2..(dy * ow + dx) * 2 + 2]
                        .copy_from_slice(&frame[at..at + 2]);
                }
            }
            expected.extend(plane);
            source_offset += sw * sh * 2;
        }
    }
    assert_eq!(&bytes[end..], expected);
}

#[test]
fn transformed_y4m_retains_range_and_display_aspect_when_resized() {
    use fvid::native_geometry::{Transpose, VideoGeometry};
    let dir = directory();
    let input = dir.0.join("source.y4m");
    let output = dir.0.join("out.y4m");
    let mut bytes = b"YUV4MPEG2 W4 H2 F25:1 Ip A2:1 C444 XCOLORRANGE=FULL\nFRAME\n".to_vec();
    bytes.extend(0..8);
    bytes.extend([128; 16]);
    std::fs::write(&input, bytes).unwrap();
    let geometry = VideoGeometry {
        transpose: Some(Transpose::Clock),
        scale: Some([4, 4]),
        ..Default::default()
    };
    assert_eq!(
        fvid::native_export::export_y4m_transformed(&input, &output, None, &geometry).unwrap(),
        1
    );
    let bytes = std::fs::read(&output).unwrap();
    let end = bytes.iter().position(|&b| b == b'\n').unwrap() + 1;
    let header = std::str::from_utf8(&bytes[..end]).unwrap();
    assert!(
        header.contains("W4 H4 F25:1 Ip A1:4 C444 XCOLORRANGE=FULL"),
        "{header}"
    );
    assert_eq!(&bytes[end..end + 6], b"FRAME\n");
    assert_eq!(
        &bytes[end + 6..end + 22],
        &[4, 4, 0, 0, 5, 5, 1, 1, 6, 6, 2, 2, 7, 7, 3, 3]
    );
    let mut reader =
        fvid::playback_native::NativeReader::software(std::io::Cursor::new(bytes), usize::MAX)
            .unwrap();
    assert_eq!(reader.pixel_aspect(), (1, 4));
    assert!(reader.read_frame_raw().unwrap().is_some());
}

#[test]
fn invalid_export_geometry_and_unrepresentable_chroma_publish_nothing() {
    use fvid::native_geometry::{Transpose, VideoGeometry};
    let dir = directory();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/geometry/422.y4m");
    for geometry in [
        VideoGeometry {
            crop: Some([2, 0, 4, 2]),
            ..Default::default()
        },
        VideoGeometry {
            scale: Some([0, 2]),
            ..Default::default()
        },
        VideoGeometry {
            transpose: Some(Transpose::Clock),
            ..Default::default()
        },
    ] {
        assert!(
            fvid::native_export::export_y4m_transformed(
                &source,
                &dir.0.join("out.y4m"),
                None,
                &geometry
            )
            .is_err()
        );
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
    }
}

#[test]
fn user_turn_can_undo_container_display_rotation_before_export() {
    use fvid::native_geometry::{Transpose, VideoGeometry};
    let dir = directory();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/display/par-2x1.mp4");
    let baseline = dir.0.join("baseline.y4m");
    fvid::native_export::export_y4m(&fixture, &baseline).unwrap();
    let mut mp4 = std::fs::read(&fixture).unwrap();
    let matrix = mp4.windows(4).position(|w| w == b"tkhd").unwrap() + 44;
    for (offset, value) in [(0, 0i32), (4, 65536), (12, -65536), (16, 0)] {
        mp4[matrix + offset..matrix + offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    let source = dir.0.join("rotated.mp4");
    std::fs::write(&source, mp4).unwrap();
    let output = dir.0.join("undone.y4m");
    let geometry = VideoGeometry {
        transpose: Some(Transpose::CClock),
        ..Default::default()
    };
    fvid::native_export::export_y4m_transformed(&source, &output, None, &geometry).unwrap();
    assert_eq!(
        std::fs::read(output).unwrap(),
        std::fs::read(baseline).unwrap()
    );
    let plain = fvid::native_media::decode_video(&fixture).unwrap();
    let turned = fvid::native_media::decode_video_transformed(&source, None, &geometry).unwrap();
    assert_eq!(
        (plain.width, plain.height, plain.video_frames),
        (turned.width, turned.height, turned.video_frames)
    );
}
