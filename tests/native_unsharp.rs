use std::{io::Cursor, path::PathBuf};
#[test]
fn owned_unsharp_export_retains_precision_and_known_impulse_response() {
    for (depth, expected) in [
        (8, [6u16, 13, 6, 13, 25, 13, 6, 13, 6]),
        (10, [25, 50, 25, 50, 100, 50, 25, 50, 25]),
        (16, [1600, 3200, 1600, 3200, 6400, 3200, 1600, 3200, 1600]),
    ] {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/unsharp-impulse-{depth}.y4m"
        ));
        let output =
            std::env::temp_dir().join(format!("fvid-unsharp-{}-{depth}.mkv", std::process::id()));
        let request = fvid::media_info::LosslessTransform {
            unsharp: Some("3:3:-1:3:3:0".into()),
            ..Default::default()
        };
        assert!(fvid::native_lossless::supports(&request));
        assert!(!fvid::native_lossless::identity(&request));
        let stats =
            fvid_media::transcode_lossless(&source, &output, request, &Default::default()).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.video_frames, 1);
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(&output).unwrap()),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), 1);
        let frame = fvid_media::owned_ffv1_decoder::Decoder::new(3, 3, 1 << 20)
            .unwrap()
            .decode(&reader.read_packet(0).unwrap())
            .unwrap();
        assert_eq!(frame.depth, depth);
        let samples: Vec<u16> = if depth == 8 {
            frame.frame.data.into_iter().map(u16::from).collect()
        } else {
            frame
                .frame
                .data
                .as_chunks::<2>().0.iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect()
        };
        assert_eq!(&samples[..9], &expected);
        assert_eq!(&samples[9..], &[1 << (depth - 1); 18]);
        std::fs::remove_file(output).unwrap();
    }
}
#[test]
fn mp4_request_and_cli_use_owned_unsharp() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let request = fvid::media_info::DecodeTransform {
        unsharp: Some("3:3:0.5".into()),
        ..Default::default()
    };
    let stats = fvid::native_media::decode_video_request(&source, &request).unwrap();
    assert_eq!((stats.backend, stats.video_frames), ("fvid", 25));
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode"])
        .arg(source)
        .args(["--unsharp", "3:3:0.5"])
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
