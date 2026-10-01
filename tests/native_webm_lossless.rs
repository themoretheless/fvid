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
        #[cfg(feature = "media")]
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
        if let Some(ffmpeg) = std::env::var_os("FVID_REFERENCE_FFMPEG") {
            let decode = |path: &Path| {
                let result = std::process::Command::new(&ffmpeg)
                    .args(["-v", "error", "-i"])
                    .arg(path)
                    .args([
                        "-map",
                        "0:v:0",
                        "-fps_mode",
                        "passthrough",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        stats.pixel_format.as_str(),
                        "-",
                    ])
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                result.stdout
            };
            let a = decode(&source);
            let b = decode(&output);
            assert_eq!(a.len(), b.len(), "independent decoder length: {name}");
            let mismatch = a.iter().zip(&b).position(|(a, b)| a != b);
            assert!(
                mismatch.is_none(),
                "independent decoder mismatch: {name} byte {mismatch:?}"
            );
        }
        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn admission_keeps_unsupported_audio_and_stored_crop_on_existing_path() {
    for name in ["audio/vorbis-stereo.webm", "display/crops.mkv"] {
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
        #[cfg(feature = "media")]
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
        #[cfg(not(feature = "media"))]
        let stats = fvid::native_export::transcode_ffv1_transformed(
            &source, &output, &geometry, &filters, None, None,
        )
        .unwrap();
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
        if let Some(ffmpeg) = std::env::var_os("FVID_REFERENCE_FFMPEG") {
            let decode = |path: &Path, vf: Option<String>| {
                let mut command = std::process::Command::new(&ffmpeg);
                command.args(["-v", "error", "-i"]).arg(path);
                if let Some(vf) = vf {
                    command.args(["-vf", &vf]);
                }
                let result = command
                    .args([
                        "-map",
                        "0:v:0",
                        "-fps_mode",
                        "passthrough",
                        "-pix_fmt",
                        stats.pixel_format.as_str(),
                        "-f",
                        "rawvideo",
                        "-",
                    ])
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                result.stdout
            };
            let vf = if filtered {
                "negate".into()
            } else {
                format!("crop={}:{}:0:0,hflip", w / 2, h / 2)
            };
            let a = decode(&source, Some(vf));
            let b = decode(&output, None);
            assert_eq!(a.len(), b.len());
            assert!(a.iter().zip(&b).all(|(a, b)| a == b));
        }
    }
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn aac_companions_keep_payloads_timestamps_and_decoded_samples() {
    companions_oracle("vp9/adaptive.webm", None);
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn opus_and_aac_companions_keep_timing_priming_and_samples() {
    for duration in ["5", "20", "60"] {
        companions_oracle("vp9/adaptive.webm", Some(duration));
    }
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn avc_hevc_matroska_transcode_owns_video_and_retains_audio() {
    for video in [
        "video.mp4",
        "hevc/main-ipb.mp4",
        "hevc/main10-ipb.mp4",
        "hevc/hdr10.mp4",
    ] {
        companions_oracle(video, Some("20"));
    }
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn ffv1_matroska_transcode_retains_precision_and_audio() {
    for video in ["owned-ffv1/video.mp4", "owned-ffv1/hevc/main10-ipb.mp4"] {
        companions_oracle(video, None);
    }
}

fn companions_oracle(video: &str, opus_frame: Option<&str>) {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let dir = std::env::temp_dir().join(format!(
        "fvid-webm-aac-{}-{}-{}",
        std::process::id(),
        opus_frame.unwrap_or("aac"),
        video.replace('/', "-")
    ));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let source = dir.join("source.mkv");
    let output = dir.join("output.mkv");
    let input_video = if let Some(original) = video.strip_prefix("owned-ffv1/") {
        let own = dir.join("video-input.mkv");
        fvid::native_export::transcode_ffv1_transformed(
            &root.join(original),
            &own,
            &Default::default(),
            &Default::default(),
            None,
            None,
        )
        .unwrap();
        own
    } else {
        root.join(video)
    };
    let mut command = std::process::Command::new(&ffmpeg);
    command
        .args(["-v", "error", "-i"])
        .arg(&input_video)
        .arg("-i")
        .arg(root.join("audio/aac-mono-44k.aac"))
        .args([
            "-map",
            "1:a:0",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c",
            "copy",
            "-avoid_negative_ts",
            "make_zero",
        ]);
    if let Some(duration) = opus_frame {
        command.args(["-c:a:0", "libopus", "-frame_duration:a:0", duration]);
    }
    let result = command.arg(&source).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fvid::native_lossless_y4m::eligible(&source).unwrap());
    #[cfg(feature = "media")]
    let stats =
        fvid::media::transcode_lossless(&source, &output, Default::default(), &Default::default())
            .unwrap();
    #[cfg(not(feature = "media"))]
    let stats = fvid::native_export::transcode_ffv1_transformed(
        &source,
        &output,
        &Default::default(),
        &Default::default(),
        None,
        None,
    )
    .unwrap();
    let mut original = fvid::container::webm::WebmReader::open(
        BufReader::new(File::open(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    let mut encoded = fvid::container::webm::WebmReader::open(
        BufReader::new(File::open(&output).unwrap()),
        Default::default(),
    )
    .unwrap();
    original.scan_all().unwrap();
    encoded.scan_all().unwrap();
    assert_eq!(original.tracks.len(), encoded.tracks.len());
    assert_eq!(stats.backend, "fvid");
    let video_in = original
        .packets
        .iter()
        .filter(|p| p.track == original.tracks[1].number && !p.invisible)
        .map(|p| p.pts_ns)
        .min()
        .unwrap();
    let video_out = encoded
        .packets
        .iter()
        .filter(|p| p.track == encoded.tracks[1].number)
        .map(|p| p.pts_ns)
        .min()
        .unwrap();
    assert_eq!(video_in, video_out);

    let mut copied = 0;
    for index in [0, 2] {
        let a = &original.tracks[index];
        let b = &encoded.tracks[index];
        assert_eq!(a.codec, b.codec);
        assert_eq!(a.codec_private, b.codec_private);
        assert_eq!(a.codec_delay_ns, b.codec_delay_ns);
        let input: Vec<_> = original
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == a.number)
            .map(|(i, p)| (i, p.clone()))
            .collect();
        let out: Vec<_> = encoded
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == b.number)
            .map(|(i, p)| (i, p.clone()))
            .collect();
        assert_eq!(input.len(), out.len());
        copied += input.len() as u64;
        for ((i, a), (j, b)) in input.into_iter().zip(out) {
            assert_eq!(a.pts_ns, b.pts_ns);
            assert_eq!(a.discard_padding_ns, b.discard_padding_ns);
            assert_eq!(
                original.read_packet(i).unwrap(),
                encoded.read_packet(j).unwrap()
            );
        }
        let decode = |path: &Path| {
            let result = std::process::Command::new(&ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(path)
                .args(["-map", &format!("0:{index}"), "-f", "f32le", "-"])
                .output()
                .unwrap();
            assert!(result.status.success());
            result.stdout
        };
        assert_eq!(decode(&source), decode(&output));
    }
    let video_decode = |path: &Path| {
        let result = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:v:0",
                "-fps_mode",
                "passthrough",
                "-pix_fmt",
                stats.pixel_format.as_str(),
                "-f",
                "rawvideo",
                "-",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        result.stdout
    };
    let a = video_decode(&source);
    let b = video_decode(&output);
    assert_eq!(a.len(), b.len(), "{video}");
    assert!(
        a.iter().zip(&b).all(|(a, b)| a == b),
        "independent video mismatch: {video}"
    );
    assert_eq!(stats.copied_packets, copied);
    let cancel = fvid::media_control::CancelFlag::new();
    let signal = cancel.clone();
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = completed.clone();
    let hook = fvid::media_control::ProgressHook::new(move |event| {
        if event.done {
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        if event.packets > 0 {
            signal.cancel();
        }
    });
    let cancelled = dir.join("cancelled.mkv");
    assert!(
        fvid::native_export::transcode_ffv1_transformed(
            &source,
            &cancelled,
            &Default::default(),
            &Default::default(),
            Some(&cancel),
            Some(&hook)
        )
        .is_err()
    );
    assert!(!cancelled.exists());
    assert!(!completed.load(std::sync::atomic::Ordering::SeqCst));
}
