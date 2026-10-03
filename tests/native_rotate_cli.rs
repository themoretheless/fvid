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
    drop(reader);
    std::fs::remove_dir_all(directory).unwrap();
}
