use std::path::PathBuf;
const SRT: &str = "1\r\n00:00:00,123 --> 00:00:01,234\r\n<b>Hello</b>\r\nworld\r\n\r\n2\r\n00:00:00,500 --> 00:00:02,000\r\nПривет, мир\r\n";
#[test]
fn srt_convert_cli_and_api_use_owned_ass_muxer_atomically() {
    let directory =
        std::env::temp_dir().join(format!("fvid-subtitle-convert-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let input = directory.join("input.srt");
    std::fs::write(&input, SRT).unwrap();
    let output = directory.join("native.mkv");
    let stats = fvid::native_subtitle::try_convert_srt(&input, &output, &[])
        .unwrap()
        .unwrap();
    assert_eq!(
        (stats.backend, stats.cues, stats.packets_out),
        ("fvid", 2, 2)
    );
    let mut reader = fvid::container::webm::WebmReader::open(
        std::fs::File::open(&output).unwrap(),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks[0].codec, "S_TEXT/ASS");
    assert_eq!(
        reader.read_packet(0).unwrap(),
        b"0,0,Default,,0,0,0,,{\\b1}Hello{\\b0}\\Nworld"
    );
    assert_eq!(reader.packets.len(), 2);
    let expected = std::fs::read(&output).unwrap();
    assert!(fvid::native_subtitle::try_convert_srt(&input, &output, &[]).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), expected);
    let cli = directory.join("cli.mkv");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "convert-subtitles"])
        .arg(&input)
        .arg(&cli)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read(cli).unwrap(), expected);
    #[cfg(feature = "media")]
    {
        let api = directory.join("api.mkv");
        assert_eq!(
            fvid::media::convert_subtitles(&input, &api, &Default::default())
                .unwrap()
                .backend,
            "fvid"
        );
        assert_eq!(std::fs::read(api).unwrap(), expected);
    }
    let bad = directory.join("bad.mkv");
    assert!(fvid::native_subtitle::try_convert_srt(&input, &bad, &[1]).is_err());
    for text in [
        "1\n00:00:99,000 --> 00:01:00,000\ntext",
        "1\n00:00:01,000 --> 00:00:00,000\ntext",
        "1\n00:00:00,000 --> 00:00:01,000\n",
        "1\n999999999999999999999:00:00,000 --> 999999999999999999999:00:01,000\ntext",
    ] {
        std::fs::write(&input, text).unwrap();
        assert!(fvid::native_subtitle::try_convert_srt(&input, &bad, &[]).is_err());
        assert!(!bad.exists());
    }
    std::fs::write(
        &input,
        "1\n00:00:00,000 --> 00:00:01,000\n<font color=red>text</font>",
    )
    .unwrap();
    assert!(
        fvid::native_subtitle::try_convert_srt(&input, &bad, &[])
            .unwrap()
            .is_none()
    );
    assert!(!bad.exists());
    assert!(!std::fs::read_dir(&directory).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-subtitle")
    }));
}
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn independent_decoder_preserves_srt_cue_times_text_and_style() {
    let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let input =
        std::env::temp_dir().join(format!("fvid-subtitle-oracle-{}.srt", std::process::id()));
    let output = input.with_extension("mkv");
    std::fs::write(&input, SRT).unwrap();
    fvid::native_subtitle::try_convert_srt(&input, &output, &[])
        .unwrap()
        .unwrap();
    let result = std::process::Command::new(binary)
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args(["-map", "0:s:0", "-f", "srt", "-"])
        .output()
        .unwrap();
    std::fs::remove_file(input).unwrap();
    std::fs::remove_file(output).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("00:00:00,123 --> 00:00:01,234"), "{text}");
    assert!(text.contains("<b>Hello</b>\nworld"), "{text}");
    assert!(text.contains("Привет, мир"), "{text}");
}
