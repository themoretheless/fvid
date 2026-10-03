use std::{io::Cursor, path::PathBuf};
#[test]
fn public_lossless_eq_export_preserves_all_planes_and_cli_uses_owned_backend() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/eq-ramp.y4m");
    let request = fvid::media_info::LosslessTransform {
        eq: Some("contrast=0:saturation=0".into()),
        ..Default::default()
    };
    assert!(fvid::native_lossless::supports(&request));
    assert!(!fvid::native_lossless::identity(&request));
    for cli in [false, true] {
        let output = std::env::temp_dir().join(format!("fvid-eq-{}-{cli}.mkv", std::process::id()));
        if cli {
            let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
                .args(["media", "transcode-lossless"])
                .arg(&source)
                .arg(&output)
                .args(["--eq", "contrast=0:saturation=0"])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()["backend"],
                "fvid"
            );
        } else {
            assert_eq!(
                fvid_media::transcode_lossless(
                    &source,
                    &output,
                    request.clone(),
                    &Default::default()
                )
                .unwrap()
                .backend,
                "fvid"
            );
        }
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(&output).unwrap()),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), 1);
        let frame = fvid_media::owned_ffv1_decoder::Decoder::new(16, 16, 1 << 20)
            .unwrap()
            .decode(&reader.read_packet(0).unwrap())
            .unwrap();
        assert_eq!(frame.frame.data, vec![127; 768]);
        std::fs::remove_file(output).unwrap();
    }
}
#[test]
fn mp4_decode_eq_request_and_cli_do_not_require_external_codec() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let request = fvid::media_info::DecodeTransform {
        eq: Some("brightness=0.06:contrast=1.2:gamma=1.1".into()),
        ..Default::default()
    };
    let stats = fvid::native_media::decode_video_request(&source, &request).unwrap();
    assert_eq!((stats.backend, stats.video_frames), ("fvid", 25));
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode"])
        .arg(&source)
        .args(["--eq", request.eq.as_ref().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()["backend"],
        "fvid"
    );
}

#[test]
fn owned_eq_preserves_monochrome_ffv1_and_packet_clock() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ffv1-gray-8.mkv");
    let output = std::env::temp_dir().join(format!("fvid-gray-eq-{}.mkv", std::process::id()));
    let request = fvid::media_info::LosslessTransform {
        eq: Some("contrast=0:gamma_r=2:gamma_b=2".into()),
        ..Default::default()
    };
    let stats =
        fvid_media::transcode_lossless(&source, &output, request, &Default::default()).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.pixel_format, "gray");
    assert_eq!(stats.video_frames, 2);
    let mut original = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    original.scan_all().unwrap();
    let mut exported = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&output).unwrap()),
        Default::default(),
    )
    .unwrap();
    exported.scan_all().unwrap();
    assert_eq!(exported.packets.len(), original.packets.len());
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 3, 1 << 20).unwrap();
    for index in 0..exported.packets.len() {
        assert_eq!(
            exported.packets[index].pts_ns,
            original.packets[index].pts_ns
        );
        let frame = decoder
            .decode(&exported.read_packet(index).unwrap())
            .unwrap();
        assert_eq!(decoder.monochrome(), Some(true));
        assert_eq!(&frame.frame.data[..12], &[127; 12]);
        assert_eq!(&frame.frame.data[12..], &[128; 24]);
    }
    std::fs::remove_file(output).unwrap();
}
