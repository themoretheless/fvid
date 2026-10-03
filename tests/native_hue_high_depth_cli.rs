//! High-depth hue must execute in native CLI paths without external media tools.
#[test]
fn high_depth_hue_decode_plan_and_lossless_export() {
    let directory =
        std::env::temp_dir().join(format!("fvid-cli-hue-high-depth-{}", std::process::id()));
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
            "tests/fixtures/playback-errors/hue-chroma-{depth}.y4m"
        ));
        let source = source.to_str().unwrap().to_owned();
        let decoded = run(vec![
            "media".into(),
            "decode".into(),
            source.clone(),
            "--hue".into(),
            "h=90".into(),
        ]);
        assert!(decoded.contains("\"video_frames\":3"));
        let output = directory.join(format!("hue-{depth}.mkv"));
        let plan = run(vec![
            "media".into(),
            "plan".into(),
            "transcode-lossless".into(),
            source.clone(),
            "--hue".into(),
            "h=90".into(),
        ]);
        assert!(plan.contains("hue"));
        run(vec![
            "media".into(),
            "transcode-lossless".into(),
            source,
            output.to_str().unwrap().into(),
            "--hue".into(),
            "h=90".into(),
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
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            assert_eq!(&samples[..9], &[16, 100, 200, 300, 400, 500, 600, 700, 800]);
            let center = 1u16 << (depth - 1);
            assert!(samples[9..18].iter().all(|&n| n == center + 200));
            assert!(samples[18..].iter().all(|&n| n == center + 100));
            assert_eq!(reader.packets[index].pts_ns, index as i64 * 500_000_000);
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
