//! Explicit lossless WebM companion audio reference comparisons.
use std::{fs::File, io::BufReader, path::Path};
fn aac_companions_keep_payloads_timestamps_and_decoded_samples() {
    companions_oracle("vp9/adaptive.webm", None);
}

fn opus_and_aac_companions_keep_timing_priming_and_samples() {
    for duration in ["5", "20", "60"] {
        companions_oracle("vp9/adaptive.webm", Some(duration));
    }
}

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

fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("Set FVID_REFERENCE_FFMPEG for this explicit reference benchmark");
    aac_companions_keep_payloads_timestamps_and_decoded_samples();
    opus_and_aac_companions_keep_timing_priming_and_samples();
    avc_hevc_matroska_transcode_owns_video_and_retains_audio();
    ffv1_matroska_transcode_retains_precision_and_audio();
    println!("WebM AAC and Opus companion reference comparisons passed");
}
