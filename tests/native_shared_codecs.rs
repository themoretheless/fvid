//! The frontend and standalone media API consume the same native video decoders.
use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
#[test]
fn public_video_types_and_errors_keep_their_identity() {
    let core = fvid_codecs::codec::vp9_decoder::Decoder::new(usize::MAX);
    let root: fvid::codec::vp9_decoder::Decoder = core;
    let _library: fvid_media::owned_codecs::vp9_decoder::Decoder = root;
    let signal = fvid_codecs::color::hdr::ColourDescription::default();
    let _root: fvid::color::hdr::ColourDescription = signal;
    let error: fvid::Error =
        fvid_control::error::BitstreamError("synthetic codec error".into()).into();
    assert!(matches!(error,fvid::Error::Invalid(ref text) if text=="synthetic codec error"));
    let _same: fvid_codecs::Error = error;
}
#[test]
fn standalone_and_root_plain_webm_decode_use_owned_kernels() {
    for source in [
        "vp9/adaptive.webm",
        "vp9/adaptive10.webm",
        "vp9/motion.webm",
        "vp9/odd10.webm",
        "playback-errors/shared-av1-private.webm",
    ] {
        let root = fvid::media::decode_video(&fixture(source)).unwrap();
        let library =
            fvid_media::decode_video(&fixture(source)).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(library.backend, "owned WebM compressed video decode");
        assert_eq!(library.video_frames, root.video_frames, "{source}");
        assert_eq!(
            (library.width, library.height),
            (root.width, root.height),
            "{source}"
        );
        assert_eq!(library.decode_errors, 0);
    }
}
#[test]
fn standalone_dispatch_applies_webm_transforms() {
    let request = fvid::media::DecodeTransform {
        deband: Some("1thr=.5".into()),
        ..Default::default()
    };
    let stats =
        fvid_media::decode_video_transformed(&fixture("vp9/adaptive.webm"), request).unwrap();
    assert_eq!(stats.backend, "owned WebM compressed video pipeline");
    assert!(stats.video_frames > 0);
}

#[test]
fn av1_private_header_fixture_exercises_the_old_admission_failure() {
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        std::io::Cursor::new(
            std::fs::read(fixture("playback-errors/shared-av1-private.webm")).unwrap(),
        ),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    let video = reader.tracks.iter().find(|t| t.kind == 1).unwrap();
    assert_eq!(video.codec, "V_AV1");
    assert!(!video.codec_private.is_empty());
    let stats =
        fvid_media::decode_video(&fixture("playback-errors/shared-av1-private.webm")).unwrap();
    assert!(stats.video_frames > 0);
    assert_eq!(stats.decode_errors, 0);
}
#[test]
fn truncated_packet_is_a_decode_error_not_a_capability_refusal() {
    let error = fvid_media::decode_video(&fixture("playback-errors/shared-vp9-truncated.webm"))
        .unwrap_err();
    assert!(!error.contains("does not yet support"), "{error}");
    assert!(error.contains("VP9") || error.contains("bit"), "{error}");
    let error = fvid_media::owned_codecs::vp9_decoder::Decoder::new(usize::MAX)
        .decode(&[0])
        .err()
        .unwrap();
    assert!(matches!(error, fvid_codecs::Error::Invalid(_)));
}
#[test]
fn sequence_header_in_configuration_primes_the_actual_decoder() {
    let source = fixture("playback-errors/shared-av1-private-sequence.webm");
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        std::io::Cursor::new(std::fs::read(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    let private = reader.tracks[0].codec_private.clone();
    let packet = reader.read_packet(0).unwrap();
    assert!(!fvid_media::owned_codecs::av1::Obus::new(&packet).any(|o| o.unwrap().kind == 1));
    let mut cold = fvid_media::owned_codecs::av1_decoder::Decoder::new(usize::MAX);
    assert!(
        cold.decode_packet(&packet)
            .err()
            .unwrap()
            .to_string()
            .contains("sequence")
    );
    let mut seeded = fvid_media::owned_codecs::av1_decoder::Decoder::new(usize::MAX);
    assert!(seeded.decode_packet(&private[4..]).unwrap().is_empty());
    let frames = seeded.decode_packet(&packet).unwrap();
    assert_eq!(frames.len(), 1);
    assert!(frames[0].show);
    let stats = fvid_media::decode_video(&source).unwrap();
    assert_eq!(stats.video_frames, 1);
    assert_eq!(
        (stats.width, stats.height),
        (frames[0].picture.size[0], frames[0].picture.size[1])
    );
}

#[test]
fn standalone_mp4_decodes_owned_avc_and_hevc_access_units() {
    for name in [
        "shared-avc-baseline",
        "shared-avc-bframes",
        "shared-hevc-main",
        "shared-hevc-main10",
    ] {
        let source = fixture(&format!("playback-errors/{name}.mp4"));
        let root = fvid::media::decode_video(&source).unwrap();
        let library = fvid_media::decode_video(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(library.backend, "owned MP4 compressed video decode");
        assert_eq!(library.video_frames, root.video_frames, "{name}");
        assert_eq!(
            (library.width, library.height),
            (root.width, root.height),
            "{name}"
        );
        assert_eq!(library.pixel_format, root.pixel_format, "{name}");
        assert_eq!(library.decode_errors, 0);
    }
}

#[test]
fn mp4_edits_and_transforms_use_owned_pipeline() {
    let source = fixture("hevc/main-ipb.mp4");
    let reader = fvid_media::owned_mp4::Mp4Reader::open(
        std::io::Cursor::new(std::fs::read(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    assert!(
        !reader
            .tracks()
            .iter()
            .find(|t| t.handler == *b"vide")
            .unwrap()
            .edits
            .is_empty()
    );
    let library = fvid_media::decode_video(&source).unwrap();
    let root = fvid::media::decode_video(&source).unwrap();
    assert_eq!(library.video_frames, root.video_frames);
    assert_eq!(library.backend, "owned MP4 compressed video decode");
    let request = fvid::media::DecodeTransform {
        deband: Some("1thr=.5".into()),
        ..Default::default()
    };
    let stats = fvid_media::decode_video_transformed(
        &fixture("playback-errors/shared-avc-baseline.mp4"),
        request,
    )
    .unwrap();
    assert_eq!(stats.backend, "owned MP4 compressed video pipeline");
    assert!(stats.video_frames > 0);
}

#[test]
fn mp4_repeated_disjoint_fractional_and_leading_edits_use_owned_decode() {
    for (name, expected) in [
        ("repeat", Some(6)),
        ("disjoint", Some(6)),
        ("fractional", Some(3)),
        ("leading", Some(3)),
    ] {
        let source = fixture(&format!("playback-errors/shared-edit-{name}.mp4"));
        let root = fvid::media::decode_video(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
        let library = fvid_media::decode_video(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(library.backend, "owned MP4 compressed video decode");
        assert_eq!(library.video_frames, root.video_frames, "{name}");
        if let Some(expected) = expected {
            assert_eq!(library.video_frames, expected, "{name}");
        }
        assert_eq!(
            (library.width, library.height),
            (root.width, root.height),
            "{name}"
        );
    }
}
#[test]
fn interior_empty_edit_is_an_explicit_capability_refusal() {
    let source = fixture("playback-errors/shared-edit-interior-empty.mp4");
    let error = fvid_media::decode_video(&source).unwrap_err();
    assert!(error.contains("does not yet support"), "{error}");
    let error = fvid::media::decode_video(&source).unwrap_err();
    assert!(error.contains("empty MP4 edit inside playback"), "{error}");
}

#[test]
fn mp4_duplicate_pts_keep_one_displayed_picture_per_time() {
    for (name, expected) in [("duplicate-pts.mp4", 11), ("duplicate-pts-run.mp4", 8)] {
        let source = fixture(&format!("playback-errors/{name}"));
        let root = fvid::media::decode_video(&source).unwrap();
        let library = fvid_media::decode_video(&source).unwrap();
        assert_eq!(library.video_frames, expected, "{name}");
        assert_eq!(library.video_frames, root.video_frames, "{name}");
        assert_eq!(library.backend, "owned MP4 compressed video decode");
    }
}
