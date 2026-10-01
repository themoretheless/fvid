use fvid::{
    media_control::{CancelFlag, ProgressHook},
    native_export,
    native_geometry::VideoGeometry,
    playback_native::NativeReader,
};
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-overlay-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn samples(path: &Path) -> Vec<Vec<u8>> {
    let mut reader =
        NativeReader::software(BufReader::new(File::open(path).unwrap()), usize::MAX).unwrap();
    let mut result = vec![];
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        let [w, h] = reader.dimensions();
        result.push(
            VideoGeometry::default()
                .apply_display(&frame, w, h, reader.rotation())
                .unwrap()
                .data,
        );
    }
    result
}
#[test]
fn overlay_exports_avc_hevc_main10_exact_samples_with_atomic_publication() {
    let d = dir("samples");
    for (i, name) in [
        "video.mp4",
        "hevc/main-ipb.mp4",
        "hevc/main10-ipb.mp4",
        "audio/two-audio.mp4",
    ]
    .iter()
    .enumerate()
    {
        let source = fixture(name);
        let output = d.0.join(format!("{i}.mkv"));
        let expected = samples(&source);
        let plan = fvid::native_plan::overlay(&source, &source, 0, 0).unwrap();
        assert!(plan.steps.iter().any(|s| s.action == "overlay"));
        assert_eq!(plan.inputs.len(), 2);
        let published = output.clone();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = done.clone();
        let hook = ProgressHook::new(move |e| {
            if e.done {
                assert!(published.exists());
                seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        });
        let stats =
            native_export::overlay_video(&source, &source, &output, 0, 0, None, Some(&hook))
                .unwrap();
        assert_eq!(stats.video_frames, expected.len() as u64);
        assert_eq!(samples(&output), expected);
        assert_eq!(done.load(std::sync::atomic::Ordering::Relaxed), 1);
        let bytes = std::fs::read(&output).unwrap();
        if name.contains("two-audio") {
            assert!(stats.copied_packets > 0);
            let mut input = fvid::container::mp4::Mp4Reader::open(
                BufReader::new(File::open(&source).unwrap()),
                Default::default(),
            )
            .unwrap();
            let mut mkv = fvid::container::webm::WebmReader::open(
                std::io::Cursor::new(&bytes),
                Default::default(),
            )
            .unwrap();
            mkv.scan_all().unwrap();
            assert_eq!(input.tracks().len(), mkv.tracks.len());
            assert_eq!(input.tags(), &mkv.tags);
            for (i, track) in input.tracks().to_vec().iter().enumerate() {
                if track.handler != *b"soun" {
                    continue;
                }
                let indices: Vec<_> = mkv
                    .packets
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.track == i as u64 + 1)
                    .map(|(j, _)| j)
                    .collect();
                assert_eq!(mkv.tracks[i].codec, "A_AAC");
                assert_eq!(indices.len(), track.samples.len());
                for (j, index) in indices.into_iter().enumerate() {
                    let mut original = Vec::new();
                    input.read_packet(i, j, &mut original).unwrap();
                    assert_eq!(mkv.read_packet(index).unwrap(), original);
                }
            }
        }

        assert!(native_export::overlay_video(&source, &source, &output, 0, 0, None, None).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        if let Some(ffmpeg) = std::env::var_os("FVID_REFERENCE_FFMPEG") {
            let pixel = if name.contains("main10") {
                "yuv420p10le"
            } else {
                "yuv420p"
            };
            let result = std::process::Command::new(ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(&output)
                .args(["-f", "rawvideo", "-pix_fmt", pixel, "-"])
                .output()
                .unwrap();
            assert!(result.status.success());
            assert_eq!(result.stdout, expected.concat());
        }
        #[cfg(feature = "media")]
        {
            let public = d.0.join(format!("public-{i}.mkv"));
            fvid::media::overlay_video(&source, &source, &public, 0, 0, &Default::default())
                .unwrap();
            assert_eq!(std::fs::read(public).unwrap(), bytes);
        }
    }
}
#[test]
fn cancellation_and_unaligned_chroma_leave_no_destination_or_temporaries() {
    let d = dir("control");
    let source = fixture("video.mp4");
    let output = d.0.join("out.mkv");
    let cancel = CancelFlag::new();
    let trigger = cancel.clone();
    let hook = ProgressHook::new(move |e| {
        assert!(!e.done);
        trigger.cancel();
    });
    assert!(
        native_export::overlay_video(&source, &source, &output, 0, 0, Some(&cancel), Some(&hook))
            .is_err()
    );
    assert!(!output.exists());
    assert!(native_export::overlay_video(&source, &source, &output, 1, 0, None, None).is_err());
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 0);
}
