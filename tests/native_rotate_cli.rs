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
    drop(oblique);
    drop(reader);
    std::fs::remove_dir_all(directory).unwrap();
}
