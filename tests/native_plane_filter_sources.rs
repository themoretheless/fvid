#[test]
fn matroska_vp9_av1_plane_filters_use_owned_decode() {
    use fvid::media::{DecodeTransform, decode_video_transformed};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in [
        "vp9/adaptive.webm",
        "vp9/odd10.webm",
        "vp9/lossless12.webm",
        "av1/ramp.webm",
        "av1/tiles.webm",
        "av1/random-access.webm",
    ] {
        let source = root.join(name);
        for request in [
            DecodeTransform {
                sab: Some("ls=10".into()),
                ..Default::default()
            },
            DecodeTransform {
                smartblur: Some("lt=8".into()),
                ..Default::default()
            },
            DecodeTransform {
                avgblur: Some("1:7:1".into()),
                ..Default::default()
            },
            DecodeTransform {
                boxblur: Some("1:1".into()),
                ..Default::default()
            },
            DecodeTransform {
                chromashift: Some("cbh=1:crv=-1".into()),
                ..Default::default()
            },
            DecodeTransform {
                avgblur: Some("1:7:1".into()),
                boxblur: Some("1:1".into()),
                chromashift: Some("cbh=1:crv=-1".into()),
                ..Default::default()
            },
        ] {
            let expected = fvid::native_media::decode_video_request(&source, &request).unwrap();
            let actual = decode_video_transformed(&source, request).unwrap();
            assert_eq!(actual.backend, "fvid", "{name}");
            assert_eq!(actual.video_frames, expected.video_frames);
            assert_eq!(actual.pixel_format, expected.pixel_format);
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height)
            );
        }
    }
}
