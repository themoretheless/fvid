//! The production CLI must execute Gaussian filtering without external tools.
#[test]
fn decode_plan_and_export_gaussian_without_ffmpeg() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/gblur-impulse.y4m");
    let directory = std::env::temp_dir().join(format!("fvid-cli-gblur-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("filtered.mkv");
    let run = |args: Vec<String>| {
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
    let source = source.to_str().unwrap().to_owned();
    let filter = "sigma=1:sigmaV=0:planes=1".to_owned();
    let decoded = run(vec![
        "media".into(),
        "decode".into(),
        source.clone(),
        "--gblur".into(),
        filter.clone(),
    ]);
    assert!(decoded.contains("\"video_frames\":3"));
    let y4m = directory.join("filtered.y4m");
    run(vec![
        "media".into(),
        "export-y4m".into(),
        source.clone(),
        y4m.to_str().unwrap().into(),
        "--gblur".into(),
        filter.clone(),
    ]);
    let bytes = std::fs::read(&y4m).unwrap();
    let payload = bytes.windows(6).position(|w| w == b"FRAME\n").unwrap() + 6;
    assert_eq!(bytes[payload + 4], 102);
    assert_eq!(bytes[payload + 3], 62);
    let plan = run(vec![
        "media".into(),
        "plan".into(),
        "transcode-lossless".into(),
        source.clone(),
        "--gblur".into(),
        filter.clone(),
    ]);
    assert!(plan.contains("owned"));
    run(vec![
        "media".into(),
        "transcode-lossless".into(),
        source,
        output.to_str().unwrap().into(),
        "--gblur".into(),
        filter,
    ]);
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.packets.len(), 3);
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(10, 4, 1 << 20).unwrap();
    let frame = decoder
        .decode(&reader.read_packet(0).unwrap())
        .unwrap()
        .frame;
    assert_eq!(frame.data[4], 102);
    assert_eq!(frame.data[3], 62);
    assert!(frame.data[40..].iter().all(|v| *v == 123));
    drop(reader);
    std::fs::remove_dir_all(directory).unwrap();
}
