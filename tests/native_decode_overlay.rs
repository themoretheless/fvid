use fvid::media_info::{DecodeTransform, OverlaySpec};
use std::path::Path;

#[test]
fn owned_decode_composites_with_geometry_filters_and_intervals() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["video.mp4", "hevc/main10-ipb.mp4", "vp9/odd10.webm"] {
        let source = root.join(name);
        let plain = fvid::native_media::decode_video(&source).unwrap();
        let request = DecodeTransform {
            horizontal_flip: true,
            negate: Some("1".into()),
            overlay: Some(OverlaySpec {
                path: source.clone(),
                x: 0,
                y: 0,
            }),
            ..Default::default()
        };
        let stats = fvid::native_media::decode_video_request(&source, &request).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.video_frames, plain.video_frames);
        assert_eq!((stats.width, stats.height), (plain.width, plain.height));
        #[cfg(feature = "media")]
        {
            let api = fvid::media::decode_video_transformed(&source, request.clone()).unwrap();
            assert_eq!(api.backend, "fvid");
            assert_eq!(api.video_frames, stats.video_frames);
            let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
                .arg("media")
                .arg("decode")
                .arg(&source)
                .arg("--overlay")
                .arg(&source)
                .arg("--hflip")
                .arg("--negate")
                .arg("1")
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            let json: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
            assert_eq!(json["backend"], "fvid");
        }
        let mut interval = request;
        interval.interval = Some((0, 1_000_000));
        let stats = fvid::native_media::decode_video_request(&source, &interval).unwrap();
        assert!(stats.video_frames > 0 && stats.video_frames <= plain.video_frames);
        let mut invalid = interval;
        invalid.overlay.as_mut().unwrap().x = 1;
        assert!(fvid::native_media::decode_video_request(&source, &invalid).is_err());
    }
}
