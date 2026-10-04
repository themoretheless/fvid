use std::{fs::File, io::BufReader, path::Path};

#[test]
fn ffv1_transcode_preserves_all_encoder_plane_layouts() {
    use fvid::container::matroska_write::{Encoding, PacketWriter, TrackSpec};
    let dir = std::env::temp_dir().join(format!("fvid-transcode-layouts-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for depth in [8, 10] {
        for (sx, sy, name) in [
            (2, 2, "yuv420p"),
            (2, 1, "yuv422p"),
            (1, 1, "yuv444p"),
            (1, 2, "yuv440p"),
            (4, 1, "yuv411p"),
            (4, 4, "yuv410p"),
        ] {
            let source = dir.join(format!("source-{sx}-{sy}-{depth}.mkv"));
            let output = dir.join(format!("out-{sx}-{sy}-{depth}.mkv"));
            let samples = 16 * 16 + 2 * (16 / sx) * (16 / sy);
            let mut data = Vec::new();
            for index in 0..samples {
                let value = ((index * 37) % ((1 << depth) - 1)) as u16;
                if depth == 8 {
                    data.push(value as u8);
                } else {
                    data.extend(value.to_le_bytes());
                }
            }
            let frame = fvid::native_geometry::GeometryFrame {
                width: 16,
                height: 16,
                subsampling: Some([sx, sy]),
                data,
            };
            let payload = fvid::codec::ffv1_encoder::encode(&frame, depth).unwrap();
            let mut container = std::io::Cursor::new(Vec::new());
            let specs = [TrackSpec {
                encoding: Encoding::Ffv1V1 {
                    width: 16,
                    height: 16,
                },
                name: "",
                language: "",
            }];
            let mut writer = PacketWriter::new(&mut container, &specs).unwrap();
            writer
                .write_packet(0, 0, 40_000_000, true, &payload)
                .unwrap();
            writer.finish().unwrap();
            std::fs::write(&source, container.into_inner()).unwrap();
            let stats = fvid::native_export::transcode_ffv1_transformed(
                &source,
                &output,
                &Default::default(),
                &Default::default(),
                None,
                None,
            )
            .unwrap();
            assert_eq!(
                stats.pixel_format,
                if depth == 8 {
                    name.to_string()
                } else {
                    format!("{name}{depth}le")
                }
            );
            let mut decoded = fvid::playback_native::NativeReader::software(
                BufReader::new(File::open(output).unwrap()),
                usize::MAX,
            )
            .unwrap();
            let fvid::playback_native::RawFrame::Planar(decoded) =
                decoded.read_frame_raw().unwrap().unwrap()
            else {
                panic!("missing source-depth planes");
            };
            assert_eq!(decoded.depth, depth);
            assert_eq!(decoded.frame.data, frame.data);
        }
    }
}

#[test]
fn vp9_av1_frames_and_clock_survive_owned_ffv1_export() {
    for name in [
        "display/crops.mkv",
        "display/vp9-rot90.mkv",
        "vp9/adaptive.webm",
        "av1/ramp.webm",
        "av1/tiles.webm",
        "vp9/odd10.webm",
        "vp9/lossless12.webm",
    ] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        assert!(fvid::native_lossless_y4m::eligible(&source).unwrap());
        let output = std::env::temp_dir().join(format!(
            "fvid-webm-ffv1-{}-{}.mkv",
            std::process::id(),
            name.replace('/', "-")
        ));
        {
            let plan = fvid::media::plan_transcode_lossless(
                &source,
                &Default::default(),
                &Default::default(),
                None,
            )
            .unwrap();
            assert!(plan.graph.is_none());
            assert!(plan.notes.iter().any(|n| n.contains("backend: fvid")));
        }
        let stats = fvid::native_export::transcode_ffv1_transformed(
            &source,
            &output,
            &Default::default(),
            &Default::default(),
            None,
            None,
        )
        .unwrap();
        let mut original = fvid::playback_native::NativeReader::software(
            BufReader::new(File::open(&source).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let mut encoded = fvid::playback_native::NativeReader::software(
            BufReader::new(File::open(&output).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let a = fvid::container::webm::WebmReader::open(
            BufReader::new(File::open(&source).unwrap()),
            Default::default(),
        )
        .unwrap();
        let b = fvid::container::webm::WebmReader::open(
            BufReader::new(File::open(&output).unwrap()),
            Default::default(),
        )
        .unwrap();
        assert!(b.tracks[0].default_duration_ns > 0);
        if a.tracks[0].default_duration_ns != 0 {
            assert_eq!(
                a.tracks[0].default_duration_ns,
                b.tracks[0].default_duration_ns
            );
        }
        assert_eq!(a.tracks[0].rotation, b.tracks[0].rotation);
        assert_eq!(a.tracks[0].crop, b.tracks[0].crop);
        assert_eq!(a.tracks[0].name, b.tracks[0].name);
        assert_eq!(a.tracks[0].language, b.tracks[0].language);
        assert_eq!(a.tracks[0].pixel_aspect(), b.tracks[0].pixel_aspect());
        let mut count = 0;
        while let Some(frame) = original.read_frame_raw().unwrap() {
            let decoded = encoded
                .read_frame_raw()
                .unwrap()
                .expect("missing output frame");
            let [w, h] = original.dimensions();
            let (w, h) = if matches!(original.rotation(), 90 | 270) {
                (h, w)
            } else {
                (w, h)
            };
            let geometry = fvid::native_geometry::VideoGeometry::default();
            let a = geometry.apply(&frame, w, h).unwrap();
            let b = geometry.apply(&decoded, w, h).unwrap();
            assert_eq!(
                (a.width, a.height, a.subsampling),
                (b.width, b.height, b.subsampling)
            );
            assert_eq!(a.data, b.data);
            assert_eq!(original.frame_interval(), encoded.frame_interval());
            count += 1;
        }
        assert!(encoded.read_frame_raw().unwrap().is_none());
        assert_eq!(stats.video_frames, count);

        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn admission_keeps_unsupported_audio_on_existing_path() {
    for name in ["audio/vorbis-stereo.webm"] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        assert!(!fvid::native_lossless_y4m::eligible(&source).unwrap());
    }
}

#[test]
fn rotated_source_geometry_and_filter_bake_orientation_once() {
    use fvid::native_geometry::VideoGeometry;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/display/vp9-rot90.mkv");
    let mut reader = fvid::playback_native::NativeReader::software(
        BufReader::new(File::open(&source).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let raw = reader.read_frame_raw().unwrap().unwrap();
    let [w, h] = reader.dimensions();
    let rotation = reader.rotation();
    for filtered in [false, true] {
        let geometry = if filtered {
            VideoGeometry::default()
        } else {
            VideoGeometry {
                crop: Some([0, 0, w / 2, h / 2]),
                horizontal_flip: true,
                ..Default::default()
            }
        };
        let filters = fvid::native_pixels::PixelFilters {
            negate: filtered.then_some(fvid::native_pixels::Negate),
            ..Default::default()
        };
        let output = std::env::temp_dir().join(format!(
            "fvid-rotated-transcode-{}-{filtered}.mkv",
            std::process::id()
        ));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(output.clone());
        let stats = {
            let transform = if filtered {
                fvid::media::LosslessTransform {
                    negate: Some("0".into()),
                    ..Default::default()
                }
            } else {
                fvid::media::LosslessTransform {
                    crop: Some(fvid::media::CropRect {
                        x: 0,
                        y: 0,
                        width: w / 2,
                        height: h / 2,
                    }),
                    horizontal_flip: true,
                    ..Default::default()
                }
            };
            fvid::media::transcode_lossless(&source, &output, transform, &Default::default())
                .unwrap()
        };
        assert_eq!(stats.backend, "fvid");
        let mut expected = geometry.apply_display_media(&raw, w, h, rotation).unwrap();
        filters.apply(&mut expected, 8).unwrap();
        let mut decoded = fvid::playback_native::NativeReader::software(
            BufReader::new(File::open(&output).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let frame = decoded.read_frame_raw().unwrap().unwrap();
        assert_eq!(decoded.rotation(), 0);
        assert_eq!(decoded.dimensions(), [expected.width, expected.height]);
        let actual = VideoGeometry::default()
            .apply(&frame, expected.width, expected.height)
            .unwrap();
        assert_eq!(actual.data, expected.data);

    }
}

#[test]
fn stored_crop_then_rotation_then_filter_preserves_ten_bit_samples() {
    use fvid::{
        container::matroska_write::{
            Encoding, PacketWriter, TrackOptions, TrackSpec, VideoMetadata,
        },
        native_geometry::{GeometryFrame, VideoGeometry},
    };
    let dir = std::env::temp_dir().join(format!("fvid-crop-rotate-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let (w, h) = (20usize, 18usize);
    let mut data = Vec::new();
    for n in 0..w * h + 2 * w.div_ceil(2) * h.div_ceil(2) {
        data.extend(((n * 37 % 1024) as u16).to_le_bytes());
    }
    let frame = GeometryFrame {
        width: w,
        height: h,
        subsampling: Some([2, 2]),
        data,
    };
    let packet = fvid::codec::ffv1_encoder::encode(&frame, 10).unwrap();
    let mut bytes = std::io::Cursor::new(Vec::new());
    let spec = [TrackSpec {
        encoding: Encoding::Ffv1V1 {
            width: w as u32,
            height: h as u32,
        },
        name: "cropped",
        language: "und",
    }];
    let options = [TrackOptions {
        rotation: 90,
        video: Some(VideoMetadata {
            crop: [2, 2, 4, 4],
            ..Default::default()
        }),
        ..Default::default()
    }];
    let mut writer = PacketWriter::new_with_options(&mut bytes, &spec, &options).unwrap();
    writer
        .write_packet(0, 0, 40_000_000, true, &packet)
        .unwrap();
    writer.finish().unwrap();
    let source = dir.join("source.mkv");
    std::fs::write(&source, bytes.into_inner()).unwrap();
    for filtered in [false, true] {
        let output = dir.join(format!("out-{filtered}.mkv"));
        let filters = fvid::native_pixels::PixelFilters {
            negate: filtered.then_some(fvid::native_pixels::Negate),
            ..Default::default()
        };
        let stats = fvid::native_export::transcode_ffv1_transformed(
            &source,
            &output,
            &VideoGeometry::default(),
            &filters,
            None,
            None,
        )
        .unwrap();
        let mut decoded = fvid::playback_native::NativeReader::software(
            BufReader::new(File::open(&output).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let raw = decoded.read_frame_raw().unwrap().unwrap();
        let fvid::playback_native::RawFrame::Planar(decoded_frame) = raw else {
            panic!("missing original precision");
        };
        if !filtered {
            assert_eq!(decoded_frame.frame.data, frame.data);
            assert_eq!(decoded.rotation(), 90);
            let metadata = fvid::container::webm::WebmReader::open(
                BufReader::new(File::open(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            assert_eq!(metadata.tracks[0].crop, [2, 2, 4, 4]);
        } else {
            assert_eq!(decoded.rotation(), 0);
            assert_eq!(decoded.insets(), [0; 4]);
            assert_eq!(decoded.dimensions(), [12, 14]);

        }
    }
}
