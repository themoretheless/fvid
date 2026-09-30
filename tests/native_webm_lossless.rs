use std::{fs::File, io::BufReader, path::Path};

#[test]
fn vp9_av1_frames_and_clock_survive_owned_ffv1_export() {
    for name in [
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
            let a=decode(&source);let b=decode(&output);
            assert_eq!(a.len(),b.len(),"independent decoder length: {name}");
            let mismatch=a.iter().zip(&b).position(|(a,b)|a!=b);
            assert!(mismatch.is_none(),"independent decoder mismatch: {name} byte {mismatch:?}");
        }
        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn admission_keeps_audio_and_display_transforms_on_existing_path() {
    for name in [
        "audio/vorbis-stereo.webm",
        "display/vp9-rot90.mkv",
        "display/crops.mkv",
    ] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        assert!(!fvid::native_lossless_y4m::eligible(&source).unwrap());
    }
}
