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
        assert!(b.tracks[0].default_duration_ns > 0);
        if a.tracks[0].default_duration_ns != 0 {
            assert_eq!(
                a.tracks[0].default_duration_ns,
                b.tracks[0].default_duration_ns
            );
        }
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

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn aac_companions_keep_payloads_timestamps_and_decoded_samples() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let dir = std::env::temp_dir().join(format!("fvid-webm-aac-{}", std::process::id()));
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
    let result = std::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(root.join("vp9/adaptive.webm"))
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
        ])
        .arg(&source)
        .output()
        .unwrap();
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
