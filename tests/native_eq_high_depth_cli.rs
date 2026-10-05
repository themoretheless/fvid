//! High-depth eq must execute in native CLI paths without external media tools.
#[test]
fn high_depth_eq_decode_plan_and_lossless_export() {
    let directory =
        std::env::temp_dir().join(format!("fvid-cli-eq-high-depth-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
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
    for depth in [12, 16] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/eq-precision-{depth}.y4m"
        ));
        let source = source.to_str().unwrap().to_owned();
        let decoded = run(vec![
            "media".into(),
            "decode".into(),
            source.clone(),
            "--eq".into(),
            "gamma=2:saturation=0".into(),
        ]);
        assert!(decoded.contains("\"video_frames\":3"));
        let output = directory.join(format!("eq-{depth}.mkv"));
        let plan = run(vec![
            "media".into(),
            "plan".into(),
            "transcode-lossless".into(),
            source.clone(),
            "--eq".into(),
            "gamma=2:saturation=0".into(),
        ]);
        assert!(plan.contains("eq"));
        run(vec![
            "media".into(),
            "transcode-lossless".into(),
            source,
            output.to_str().unwrap().into(),
            "--eq".into(),
            "gamma=2:saturation=0".into(),
        ]);
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(3, 3, 1 << 20).unwrap();
        assert_eq!(reader.packets.len(), 3);
        for index in 0..3 {
            let frame = decoder.decode(&reader.read_packet(index).unwrap()).unwrap();
            assert_eq!(frame.depth, depth);
            let samples: Vec<u16> = frame
                .frame
                .data
                .as_chunks::<2>().0.iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            let count = 1u32 << depth;
            assert_eq!(samples[0], 0);
            assert_eq!(samples[1], if depth == 12 { 64 } else { 256 });
            assert_eq!(samples[2], if depth == 12 { 90 } else { 362 });
            assert_eq!(samples[3], (count / 2) as u16);
            assert_eq!(samples[4], (count - 1) as u16);
            assert!(samples[9..].iter().all(|&n| n == (count / 2) as u16));
            assert_eq!(reader.packets[index].pts_ns, index as i64 * 500_000_000);
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
