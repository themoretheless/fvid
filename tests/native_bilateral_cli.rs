//! The production CLI must execute bilateral filtering without external tools.
#[test]
fn decode_plan_and_export_bilateral_without_ffmpeg() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/bilateral-noise-edge.y4m");
    let directory = std::env::temp_dir().join(format!("fvid-cli-bilateral-{}", std::process::id()));
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
    let owned_plan = fvid::native_plan::transcode_lossless(
        &source,
        &fvid::media_info::LosslessTransform {
            bilateral: Some("sigmaS=1:sigmaR=0.1:planes=1".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        owned_plan
            .steps
            .iter()
            .any(|step| step.action == "filter" && step.detail.contains("bilateral"))
    );
    let capabilities = run(vec!["media".into(), "capabilities".into()]);
    let capabilities: serde_json::Value = serde_json::from_str(&capabilities).unwrap();
    assert!(
        capabilities["filters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "bilateral")
    );
    let source = source.to_str().unwrap().to_owned();
    let filter = "sigmaS=1:sigmaR=0.1:planes=1".to_owned();
    let decoded = run(vec![
        "media".into(),
        "decode".into(),
        source.clone(),
        "--bilateral".into(),
        filter.clone(),
    ]);
    assert!(decoded.contains("\"video_frames\":3"));
    let y4m = directory.join("filtered.y4m");
    run(vec![
        "media".into(),
        "export-y4m".into(),
        source.clone(),
        y4m.to_str().unwrap().into(),
        "--bilateral".into(),
        filter.clone(),
    ]);
    let bytes = std::fs::read(&y4m).unwrap();
    let payload = bytes.windows(6).position(|w| w == b"FRAME\n").unwrap() + 6;
    assert!(
        bytes[payload..payload + 4]
            .iter()
            .all(|&v| (61..=63).contains(&v))
    );
    assert!(
        bytes[payload + 4..payload + 8]
            .iter()
            .all(|&v| (201..=203).contains(&v))
    );
    let plan = run(vec![
        "media".into(),
        "plan".into(),
        "transcode-lossless".into(),
        source.clone(),
        "--bilateral".into(),
        filter.clone(),
    ]);
    assert!(plan.contains("owned"));
    run(vec![
        "media".into(),
        "transcode-lossless".into(),
        source,
        output.to_str().unwrap().into(),
        "--bilateral".into(),
        filter,
    ]);
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.packets.len(), 3);
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(8, 4, 1 << 20).unwrap();
    let frame = decoder
        .decode(&reader.read_packet(0).unwrap())
        .unwrap()
        .frame;
    for row in frame.data[..32].as_chunks::<8>().0 {
        assert!(row[..4].iter().all(|&v| (61..=63).contains(&v)));
        assert!(row[4..].iter().all(|&v| (201..=203).contains(&v)));
    }
    assert!(frame.data[32..].iter().all(|v| *v == 128));
    drop(reader);
    std::fs::remove_dir_all(directory).unwrap();
}
