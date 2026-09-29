use fvid::{
    codec::ffv1_encoder,
    color::hdr::ColourDescription,
    container::matroska_write::{Encoding, PacketWriter, TrackOptions, TrackSpec, VideoMetadata},
    native_geometry::GeometryFrame,
    playback_native::{AvcColour, NativeReader, PackedPlanar, RawFrame, rotate_plane},
};
use std::{io::Cursor, time::Duration};
fn image(w: usize, h: usize, sx: usize, sy: usize, depth: u8, pattern: usize) -> GeometryFrame {
    let n = w * h + 2 * w.div_ceil(sx) * h.div_ceil(sy);
    let data = (0..n)
        .flat_map(|i| {
            let value = ((i * 97 + i * i * 3 + pattern * 317) & ((1 << depth) - 1)) as u16;
            if depth == 8 {
                vec![value as u8]
            } else {
                value.to_le_bytes().to_vec()
            }
        })
        .collect();
    GeometryFrame {
        width: w,
        height: h,
        subsampling: Some([sx, sy]),
        data,
    }
}
fn mux(frames: &[GeometryFrame], depth: u8, options: TrackOptions) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let track = TrackSpec {
        encoding: Encoding::Ffv1V1 {
            width: frames[0].width as u32,
            height: frames[0].height as u32,
        },
        name: "native",
        language: "und",
    };
    let mut writer = PacketWriter::new_with_options(&mut out, &[track], &[options]).unwrap();
    for (i, f) in frames.iter().enumerate() {
        writer
            .write_packet(
                0,
                i as u64 * 40_000_000,
                40_000_000,
                true,
                &ffv1_encoder::encode(f, depth).unwrap(),
            )
            .unwrap();
    }
    writer.finish().unwrap();
    out.into_inner()
}
#[test]
fn container_raw_preserves_every_supported_depth_layout_and_odd_size() {
    for depth in 8..=16 {
        for (sx, sy) in [(1, 1), (2, 1), (2, 2), (1, 2), (4, 1), (4, 4)] {
            let frames: Vec<_> = (0..3).map(|p| image(17, 13, sx, sy, depth, p)).collect();
            let bytes = mux(&frames, depth, Default::default());
            let mut reader = NativeReader::software(Cursor::new(bytes), 2 << 20).unwrap();
            for (i, expected) in frames.iter().enumerate() {
                let RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap() else {
                    panic!("packed frame required")
                };
                assert_eq!(p.depth, depth);
                assert_eq!(p.frame.subsampling, Some([sx, sy]));
                assert_eq!(p.frame.data, expected.data);
                assert_eq!(reader.dimensions(), [17, 13]);
                assert_eq!(
                    reader.frame_interval(),
                    Some((
                        i as u128 * 40_000_000,
                        (i as u128 + 1) * 40_000_000,
                        1_000_000_000
                    ))
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
            assert!(reader.read_frame().unwrap());
            let first = reader.rgb().to_vec();
            assert_eq!(first.len(), 17 * 13 * 3);
            reader.seek(Duration::from_millis(85)).unwrap();
            reader.seek(Duration::ZERO).unwrap();
            assert_eq!(reader.rgb(), first);
        }
    }
}
#[test]
fn container_colour_rotation_and_raw_orientation_agree() {
    for matrix in [1, 5, 7, 9] {
        for full_range in [false, true] {
            for rotation in [0, 90, 180, 270] {
                let colour = ColourDescription {
                    primaries: 9,
                    transfer: 16,
                    matrix,
                    full_range,
                };
                let frames = [image(18, 12, 2, 1, 12, 3)];
                let options = TrackOptions {
                    rotation,
                    video: Some(VideoMetadata {
                        pixel_aspect: (4, 3),
                        colour: Some(colour),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                let bytes = mux(&frames, 12, options);
                let mut reader = NativeReader::software(Cursor::new(bytes), 2 << 20).unwrap();
                let RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap() else {
                    panic!("packed frame")
                };
                assert_eq!(p.frame.data, frames[0].data);
                assert_eq!(p.colour.full, full_range);
                assert_eq!(reader.colour(), colour);
                let quarter = matches!(rotation, 90 | 270);
                assert_eq!(
                    reader.dimensions(),
                    if quarter { [12, 18] } else { [18, 12] }
                );
                assert_eq!(reader.pixel_aspect(), if quarter { (3, 4) } else { (4, 3) });
                let mut expected = Vec::new();
                p.to_rgb(&mut expected, 2 << 20).unwrap();
                if rotation != 0 {
                    expected = rotate_plane(&expected, 18, 12, rotation, 3);
                }
                reader.rewind().unwrap();
                assert!(reader.read_frame().unwrap());
                assert_eq!(reader.rgb(), expected);
                reader.seek(Duration::ZERO).unwrap();
                assert_eq!(reader.rgb(), expected);
            }
        }
    }
}
#[test]
fn packed_conversion_uses_actual_chroma_axes_and_source_precision() {
    // Equal luma, different chroma at each 4:4:4 pixel. 4:2:0 indexing would
    // repeat the first colour horizontally and vertically.
    let frame = GeometryFrame {
        width: 2,
        height: 2,
        subsampling: Some([1, 1]),
        data: vec![128, 128, 128, 128, 128, 128, 255, 128, 128, 255, 128, 0],
    };
    let p = PackedPlanar::new(frame, 8, AvcColour::default()).unwrap();
    let mut rgb = Vec::new();
    p.to_rgb(&mut rgb, 12).unwrap();
    assert_eq!(
        rgb,
        vec![130, 130, 130, 255, 27, 130, 130, 81, 255, 0, 234, 130]
    );
    assert!(p.to_rgb(&mut rgb, 11).is_err());
    assert!(p.to_planar8(11).is_err());
    let depth = 16;
    let samples = [32767u16, 32831, 32768, 32768];
    let p = PackedPlanar::new(
        GeometryFrame {
            width: 2,
            height: 1,
            subsampling: Some([2, 1]),
            data: samples.iter().flat_map(|v| v.to_le_bytes()).collect(),
        },
        depth,
        AvcColour {
            full: true,
            ..Default::default()
        },
    )
    .unwrap();
    p.to_rgb(&mut rgb, 6).unwrap();
    assert_eq!(rgb, vec![127, 127, 127, 128, 128, 128]);
    assert_eq!(p.to_planar8(4).unwrap().y, vec![127, 128]);
}
struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn media_api_and_y4m_export_keep_ffv1_samples() {
    let dir = Temp(std::env::temp_dir().join(format!("fvid-native-ffv1-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    for (depth, sx, sy, name) in [
        (8, 1, 1, "444"),
        (10, 2, 2, "420"),
        (12, 2, 1, "422"),
        (16, 1, 1, "444"),
    ] {
        let frames: Vec<_> = (0..3).map(|p| image(17, 13, sx, sy, depth, p)).collect();
        let source = dir.0.join(format!("{depth}.mkv"));
        let destination = dir.0.join(format!("{depth}.y4m"));
        std::fs::write(&source, mux(&frames, depth, Default::default())).unwrap();
        let stats = fvid::native_media::decode_video(&source).unwrap();
        assert_eq!(stats.video_frames, 3);
        assert_eq!(stats.backend, "fvid");
        assert_eq!(
            stats.pixel_format,
            if depth == 8 {
                format!("yuv{name}p")
            } else {
                format!("yuv{name}p{depth}le")
            }
        );
        #[cfg(feature = "media")]
        assert_eq!(fvid::media::decode_video(&source).unwrap().video_frames, 3);
        assert_eq!(
            fvid::native_export::export_y4m(&source, &destination).unwrap(),
            3
        );
        let data = std::fs::read(&destination).unwrap();
        let end = data.iter().position(|&b| b == b'\n').unwrap() + 1;
        let mut data = &data[end..];
        for f in &frames {
            assert!(data.starts_with(b"FRAME\n"));
            data = &data[6..];
            assert_eq!(&data[..f.data.len()], f.data);
            data = &data[f.data.len()..];
        }
        assert!(data.is_empty());
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "decode"])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn threaded_playback_and_seek_render_identical_rotated_frames() {
    use fvid::playback_thread::{Event, Pixels, Playback};
    use std::time::Instant;
    let frames: Vec<_> = (0..5).map(|p| image(18, 12, 2, 1, 12, p)).collect();
    let data = mux(
        &frames,
        12,
        TrackOptions {
            rotation: 90,
            ..Default::default()
        },
    );
    let mut reference = NativeReader::software(Cursor::new(data.clone()), 2 << 20).unwrap();
    let mut expected = Vec::new();
    while reference.read_frame().unwrap() {
        expected.push(reference.rgb().to_vec());
    }
    let mut reader = NativeReader::software(Cursor::new(data), 2 << 20).unwrap();
    assert!(reader.read_frame().unwrap());
    let mut player = Playback::start(reader, None);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut index = 0;
    loop {
        assert!(Instant::now() < deadline, "FFV1 playback stalled");
        match player.poll() {
            Some(Event::Frame(f)) => {
                assert_eq!(f.dimensions, [12, 18]);
                let Pixels::Rgb(rgb) = f.pixels else {
                    panic!("generic planar needs exact CPU presentation")
                };
                assert_eq!(rgb, expected[index]);
                index += 1;
            }
            Some(Event::Ended(_)) => break,
            Some(Event::Error(e)) => panic!("{e}"),
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    assert_eq!(index, 5);
    player.pause();
    player.seek(Duration::from_millis(85));
    let generation = player.generation();
    loop {
        assert!(Instant::now() < deadline, "FFV1 seek stalled");
        match player.poll() {
            Some(Event::Frame(f)) if f.generation == generation => {
                let Pixels::Rgb(rgb) = f.pixels else {
                    panic!("packed seek")
                };
                assert_eq!(rgb, expected[2]);
                assert_eq!(f.interval.unwrap().0, 80_000_000);
                break;
            }
            Some(Event::Error(e)) => panic!("{e}"),
            _ => std::thread::sleep(Duration::from_millis(1)),
        }
    }
}

#[test]
fn packed_geometry_transforms_high_depth_without_narrowing() {
    use fvid::native_geometry::VideoGeometry;
    use std::sync::Arc;
    let source = image(8, 4, 2, 2, 12, 3);
    let bytes = source.data.clone();
    let raw = RawFrame::Planar(Arc::new(
        PackedPlanar::new(source, 12, AvcColour::default()).unwrap(),
    ));
    let transformed = VideoGeometry {
        crop: Some([2, 0, 4, 4]),
        horizontal_flip: true,
        ..Default::default()
    }
    .apply(&raw, 8, 4)
    .unwrap();
    let mut expected = Vec::new();
    let mut offset = 0;
    for (w, h, x, crop_width) in [(8, 4, 2, 4), (4, 2, 1, 2), (4, 2, 1, 2)] {
        for y in 0..h {
            for xx in (x..x + crop_width).rev() {
                let i = offset + 2 * (y * w + xx);
                expected.extend_from_slice(&bytes[i..i + 2]);
            }
        }
        offset += w * h * 2;
    }
    assert_eq!(transformed.data, expected);
    assert_eq!(transformed.subsampling, Some([2, 2]));
}
