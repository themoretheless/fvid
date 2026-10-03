use std::{io::Cursor, path::PathBuf};
#[test]
fn constant_hue_exports_owned_ffv1_with_expected_pixels() {
    for depth in [8, 10] {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/playback-errors/hue-{depth}.y4m"));
        let output =
            std::env::temp_dir().join(format!("fvid-hue-{}-{depth}.mkv", std::process::id()));
        let stats = fvid_media::transcode(
            &source,
            &output,
            fvid::media_info::LosslessTransform {
                hue: Some("h=90".into()),
                ..Default::default()
            },
            &Default::default(),
            &fvid::media_info::EncoderSettings {
                name: "ffv1".into(),
                options: vec![("level".into(), "1".into())],
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.video_frames, 1);
        let bytes = std::fs::read(&output).unwrap();
        std::fs::remove_file(output).unwrap();
        let mut reader =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes), Default::default())
                .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), 1);
        let decoded = fvid_media::owned_ffv1_decoder::Decoder::new(2, 2, 1 << 20)
            .unwrap()
            .decode(&reader.read_packet(0).unwrap())
            .unwrap();
        let expected = if depth == 8 {
            vec![16, 64, 128, 235, 160, 160]
        } else {
            [64u16, 256, 512, 940, 640, 640]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect()
        };
        assert_eq!(decoded.frame.data, expected);
    }
}
