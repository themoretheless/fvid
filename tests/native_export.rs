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
