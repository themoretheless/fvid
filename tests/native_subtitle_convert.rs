use std::path::PathBuf;
const SRT: &str = "1\r\n00:00:00,123 --> 00:00:01,234\r\n<b>Hello</b>\r\nworld\r\n\r\n2\r\n00:00:00,500 --> 00:00:02,000\r\nПривет, мир &amp; &#x41;\r\n";
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
    assert!(fvid::media_info::SubtitleCodec::parse("unknown").is_err());
    let options = fvid::media_info::SubtitleConvertOptions {
        streams: vec![0],
        codec: fvid::media_info::SubtitleCodec::parse("ass").unwrap(),
    };
    let stats = fvid::native_subtitle::try_convert_with_options(&input, &output, &options)
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
    for (name, text) in [
        ("spaces", SRT.replace("\r\n\r\n", "\r\n \t\r\n")),
        ("carriage", SRT.replace("\r\n", "\r")),
        ("bom", format!("\u{feff}{SRT}")),
    ] {
        let variant = directory.join(format!("{name}.srt"));
        let converted = directory.join(format!("{name}.mkv"));
        std::fs::write(&variant, text).unwrap();
        fvid::native_subtitle::try_convert_srt(&variant, &converted, &[])
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read(converted).unwrap(), expected, "{name}");
    }
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
        "1\n00:00:00,000 --> 00:00:01,000\n<font color=not-a-color>text</font>",
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
    assert!(text.contains("Привет, мир & A"), "{text}");
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG to mux SubRip tracks"]
fn matroska_subrip_selection_uses_owned_conversion_and_preserves_cues() {
    let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let directory = std::env::temp_dir().join(format!("fvid-subrip-mkv-{}", std::process::id()));
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
    let source = directory.join("source.mkv");
    let mux = std::process::Command::new(&binary)
        .args(["-v", "error", "-i"])
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4"))
        .arg("-i")
        .arg(&input)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:s:0",
            "-map",
            "1:s:0",
            "-c",
            "copy",
            "-metadata:s:s:0",
            "title=First",
            "-metadata:s:s:0",
            "language=eng",
            "-metadata:s:s:1",
            "title=Second",
            "-metadata:s:s:1",
            "language=fra",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        mux.status.success(),
        "{}",
        String::from_utf8_lossy(&mux.stderr)
    );
    for (label, streams, name, language) in [
        ("default", vec![], "First", "eng"),
        ("second", vec![2], "Second", "fra"),
    ] {
        let output = directory.join(format!("{label}.mkv"));
        let stats = fvid::native_subtitle::try_convert(&source, &output, &streams)
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                stats.backend,
                stats.cues,
                stats.packets_in,
                stats.packets_out
            ),
            ("fvid", 2, 2, 2)
        );
        let mut reader = fvid::container::webm::WebmReader::open(
            std::fs::File::open(&output).unwrap(),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks.len(), 1);
        assert_eq!(reader.tracks[0].name, name);
        assert_eq!(reader.tracks[0].language, language);
        let decoded = std::process::Command::new(&binary)
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-map", "0:s:0", "-f", "srt", "-"])
            .output()
            .unwrap();
        assert!(decoded.status.success());
        let text = String::from_utf8(decoded.stdout).unwrap();
        assert!(text.contains("00:00:00,123 --> 00:00:01,234"), "{text}");
        assert!(text.contains("<b>Hello</b>\nworld"), "{text}");
        assert!(text.contains("Привет, мир & A"), "{text}");
        let cli = directory.join(format!("cli-{label}.mkv"));
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
        command
            .args(["media", "convert-subtitles"])
            .arg(&source)
            .arg(&cli);
        if !streams.is_empty() {
            command.args(["--streams", "2"]);
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read(cli).unwrap(), std::fs::read(&output).unwrap());
        #[cfg(feature = "media")]
        {
            let api = directory.join(format!("api-{label}.mkv"));
            let options = fvid::media::SubtitleConvertOptions {
                streams: streams.clone(),
                ..Default::default()
            };
            assert_eq!(
                fvid::media::convert_subtitles(&source, &api, &options)
                    .unwrap()
                    .backend,
                "fvid"
            );
            assert_eq!(std::fs::read(api).unwrap(), std::fs::read(&output).unwrap());
        }
    }
    let bad = directory.join("bad.mkv");
    assert!(
        fvid::native_subtitle::try_convert(&source, &bad, &[0])
            .err()
            .unwrap()
            .to_string()
            .contains("not a subtitle")
    );
    assert!(fvid::native_subtitle::try_convert(&source, &bad, &[9]).is_err());
    assert!(fvid::native_subtitle::try_convert(&source, &bad, &[0, 1]).is_err());
    assert!(!bad.exists());
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn font_attributes_match_independent_subrip_conversion() {
    let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let directory =
        std::env::temp_dir().join(format!("fvid-subtitle-colors-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    for (index, color) in ["ff0000", "00ff00", "0000ff", "12AbEf", "ffffff", "000000"]
        .iter()
        .enumerate()
    {
        for (nested_index, nested) in [
            "color=\"#345678\"",
            "face=\"Courier New\"",
            "size=\"18\"",
            "size=18",
            "color=red",
            "color=green",
            "color=blue",
            "color=white",
            "color=black",
            "color=silver",
            "color=gray",
            "color=maroon",
            "color=purple",
            "color=fuchsia",
            "color=lime",
            "color=olive",
            "color=yellow",
            "color=navy",
            "color=teal",
            "color=aqua",
        ]
        .iter()
        .enumerate()
        {
            // Named colors use a distinct outer color to compare serialized styles
            // without redundant same-color tags emitted by the reference.
            if nested.starts_with("color=") && !nested.contains('#') && *color != "12AbEf" {
                continue;
            }
            let source = directory.join(format!("{index}-{nested_index}.srt"));
            std::fs::write(&source,format!("1\n00:00:00,100 --> 00:00:01,000\nBefore <font color=\"#{color}\" face=\"Georgia\" size=\"24\">colored <b>bold</b> <font {nested}>nested</font> restored</font> after\n")).unwrap();
            let output = source.with_extension("mkv");
            fvid::native_subtitle::try_convert_srt(&source, &output, &[])
                .unwrap()
                .unwrap();
            let decode = |path: &std::path::Path| {
                let result = std::process::Command::new(&binary)
                    .args(["-v", "error", "-i"])
                    .arg(path)
                    .args(["-map", "0:s:0", "-f", "srt", "-"])
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                String::from_utf8(result.stdout).unwrap()
            };
            assert_eq!(decode(&output), decode(&source), "{color}: {nested}");
        }
    }
}

#[test]
fn matroska_ass_conversion_preserves_header_styling_packets_and_timing() {
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/subtitles/ass-track.mkv");
    let directory =
        std::env::temp_dir().join(format!("fvid-owned-ass-copy-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let mut input = fvid::container::webm::WebmReader::open(
        std::fs::File::open(&source).unwrap(),
        Default::default(),
    )
    .unwrap();
    input.scan_all().unwrap();
    let index = input
        .tracks
        .iter()
        .position(|t| t.codec == "S_TEXT/ASS")
        .unwrap();
    let track = input.tracks[index].clone();
    let mut expected = Vec::new();
    for i in 0..input.packets.len() {
        let packet = &input.packets[i];
        if packet.track == track.number {
            let (pts, duration) = (packet.pts_ns, packet.duration_ns);
            expected.push((pts, duration, input.read_packet(i).unwrap()));
        }
    }
    assert!(!expected.is_empty());
    for (label, selection) in [("default", vec![]), ("selected", vec![index])] {
        let output = directory.join(format!("{label}.mkv"));
        let stats = fvid::native_subtitle::try_convert(&source, &output, &selection)
            .unwrap()
            .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.cues, expected.len() as u64);
        let mut result = fvid::container::webm::WebmReader::open(
            std::fs::File::open(&output).unwrap(),
            Default::default(),
        )
        .unwrap();
        result.scan_all().unwrap();
        assert_eq!(result.tracks.len(), 1);
        assert_eq!(result.tracks[0].codec_private, track.codec_private);
        assert_eq!(result.tracks[0].name, track.name);
        assert_eq!(result.tracks[0].language, track.language);
        assert_eq!(result.packets.len(), expected.len());
        for (i, (pts, duration, payload)) in expected.iter().enumerate() {
            assert_eq!(
                (result.packets[i].pts_ns, result.packets[i].duration_ns),
                (*pts, *duration)
            );
            assert_eq!(result.read_packet(i).unwrap(), *payload);
        }
        let original = std::fs::read(&output).unwrap();
        assert!(fvid::native_subtitle::try_convert(&source, &output, &selection).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), original);
        let cli = directory.join(format!("cli-{label}.mkv"));
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
        command
            .args(["media", "convert-subtitles"])
            .arg(&source)
            .arg(&cli);
        if !selection.is_empty() {
            command.arg("--streams").arg(index.to_string());
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read(cli).unwrap(), original);
        #[cfg(feature = "media")]
        {
            let api = directory.join(format!("api-{label}.mkv"));
            assert_eq!(
                fvid::media::convert_subtitles(
                    &source,
                    &api,
                    &fvid::media_info::SubtitleConvertOptions {
                        streams: selection,
                        ..Default::default()
                    }
                )
                .unwrap()
                .backend,
                "fvid"
            );
            assert_eq!(std::fs::read(api).unwrap(), original);
        }
    }
}

#[test]
fn standalone_ass_preserves_styles_dialogue_fields_and_timestamps() {
    let directory = std::env::temp_dir().join(format!("fvid-ass-script-{}",std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup { fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);} }
    let _cleanup = Cleanup(directory.clone());
    let header = "[Script Info]\nScriptType: v4.00+\nTitle: Synthetic\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize\nStyle: Fancy,Arial,24\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n";
    let source = directory.join("source.ass");
    std::fs::write(&source,format!("{header}Dialogue: 2,0:00:02.00,0:00:03.50,Fancy,Actor,10,20,30,,Later, comma\nDialogue: 1,0:00:00.12,0:00:01.23,Fancy,,0,0,0,,{{\\b1}}Привет\\Nworld\n")).unwrap();
    let output = directory.join("owned.mkv");
    let stats = fvid::native_subtitle::try_convert(&source,&output,&[0]).unwrap().unwrap();
    assert_eq!(stats.backend,"fvid");
    assert_eq!(stats.cues,2);
    let mut reader = fvid::container::webm::WebmReader::open(std::fs::File::open(&output).unwrap(),Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks[0].codec_private,header.as_bytes());
    assert_eq!(reader.packets[0].pts_ns,120_000_000);
    assert_eq!(reader.read_packet(0).unwrap(),"1,1,Fancy,,0,0,0,,{\\b1}Привет\\Nworld".as_bytes());
    assert_eq!(reader.packets[1].pts_ns,2_000_000_000);
    assert_eq!(reader.read_packet(1).unwrap(),b"0,2,Fancy,Actor,10,20,30,,Later, comma");
    assert!(fvid::native_subtitle::try_convert(&source,&output,&[]).is_err());
    let cli = directory.join("cli.mkv");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid")).args(["media","convert-subtitles"]).arg(&source).arg(&cli).output().unwrap();
    assert!(run.status.success(),"{}",String::from_utf8_lossy(&run.stderr));
    assert_eq!(std::fs::read(cli).unwrap(),std::fs::read(&output).unwrap());
    #[cfg(feature="media")]
    {
        let api = directory.join("api.mkv");
        assert_eq!(fvid::media::convert_subtitles(&source,&api,&Default::default()).unwrap().backend,"fvid");
        assert_eq!(std::fs::read(api).unwrap(),std::fs::read(&output).unwrap());
    }
    let invalid = directory.join("invalid.mkv");
    std::fs::write(&source,format!("{header}Dialogue: 0,0:00:02.00,0:00:01.00,Fancy,,0,0,0,,Invalid\n")).unwrap();
    assert!(fvid::native_subtitle::try_convert(&source,&invalid,&[]).is_err());
    assert!(!invalid.exists());
}
