#[test]
fn owned_rotation_runs_without_libav_and_preserves_timing() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/rotate-grid.y4m");
    let directory = std::env::temp_dir().join(format!("fvid-rotate-cli-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("rotated.mkv");
    let run = |args: &[&str]| {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(args)
            .env("PATH", "/nonexistent")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    };
    let source = source.to_str().unwrap();
    let stats = run(&["media", "decode", source, "--rotate", "90"]);
    assert!(stats.contains("\"video_frames\":3"));
    let plan = run(&[
        "media",
        "plan",
        "transcode-lossless",
        source,
        "--rotate",
        "90",
    ]);
    assert!(plan.contains("rotate"));
    run(&[
        "media",
        "transcode-lossless",
        source,
        output.to_str().unwrap(),
        "--rotate",
        "90",
    ]);
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.packets.len(), 3);
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(2, 3, 1 << 20).unwrap();
    for index in 0..3 {
        assert_eq!(reader.packets[index].pts_ns, index as i64 * 500_000_000);
        assert_eq!(reader.packets[index].duration_ns, Some(500_000_000));
        let frame = decoder
            .decode(&reader.read_packet(index).unwrap())
            .unwrap()
            .frame;
        assert_eq!(&frame.data[..6], &[4, 1, 5, 2, 6, 3]);
    }
    let oblique_source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/rotate-white420.y4m");
    let oblique_output = directory.join("oblique.mkv");
    run(&[
        "media",
        "transcode-lossless",
        oblique_source.to_str().unwrap(),
        oblique_output.to_str().unwrap(),
        "--rotate",
        "45",
    ]);
    let mut oblique = fvid_media::owned_webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(&oblique_output).unwrap()),
        Default::default(),
    )
    .unwrap();
    oblique.scan_all().unwrap();
    let expected_size = fvid::media_info::RotateAngle::parse("45")
        .unwrap()
        .size(8, 8);
    assert_eq!(expected_size, (11, 11));
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(11, 11, 1 << 20).unwrap();
    let turned = decoder
        .decode(&oblique.read_packet(0).unwrap())
        .unwrap()
        .frame;
    assert_eq!(turned.data.len(), 121 + 2 * 36);
    assert_eq!(turned.data[0], 16);
    assert_eq!(turned.data[5 * 11 + 5], 235);
    assert!(turned.data[121..].iter().all(|v| *v == 128));
    let scaled_output = directory.join("oblique-scaled.mkv");
    run(&[
        "media",
        "transcode-lossless",
        oblique_source.to_str().unwrap(),
        scaled_output.to_str().unwrap(),
        "--rotate",
        "45",
        "--pad",
        "12:12:0:0",
        "--scale",
        "6:6",
    ]);
    let mut scaled = fvid_media::owned_webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(&scaled_output).unwrap()),
        Default::default(),
    )
    .unwrap();
    scaled.scan_all().unwrap();
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(6, 6, 1 << 20).unwrap();
    let resized = decoder
        .decode(&scaled.read_packet(0).unwrap())
        .unwrap()
        .frame;
    assert_eq!(resized.data.len(), 36 + 2 * 9);
    assert!(resized.data[36..].iter().all(|v| *v == 128));
    drop(scaled);
    let y4m_output = directory.join("oblique.y4m");
    run(&[
        "media",
        "export-y4m",
        oblique_source.to_str().unwrap(),
        y4m_output.to_str().unwrap(),
        "--rotate",
        "45",
    ]);
    let roundtrip = run(&["media", "decode", y4m_output.to_str().unwrap()]);
    let roundtrip: serde_json::Value = serde_json::from_str(&roundtrip).unwrap();
    assert_eq!(roundtrip["video_frames"], 3);
    assert_eq!(roundtrip["width"], 11);
    assert_eq!(roundtrip["height"], 11);
    let direct =
        fvid_media::owned_y4m_decode::decode_video_transformed(&y4m_output, Default::default())
            .unwrap();
    assert_eq!(direct.video_frames, 3);
    assert_eq!((direct.width, direct.height), (11, 11));
    drop(oblique);
    drop(reader);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn odd_y4m_reproducer_reads_all_frames_and_edge_pixels() {
    let bytes = include_bytes!("fixtures/playback-errors/rotate-odd420.y4m");
    let header =
        fvid::Header::parse(bytes.split_inclusive(|b| *b == b'\n').next().unwrap()).unwrap();
    assert_eq!(header.frame_len().unwrap(), 121 + 2 * 36);
    let mut reader = fvid::playback::Y4mReader::new(std::io::Cursor::new(bytes), 1 << 20).unwrap();
    let mut frames = 0;
    while reader.read_frame().unwrap() {
        assert_eq!(reader.rgb().len(), 11 * 11 * 3);
        assert!(reader.rgb().iter().all(|v| *v == 255));
        frames += 1;
    }
    assert_eq!(frames, 3);
}
