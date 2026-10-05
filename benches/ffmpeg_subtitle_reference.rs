//! Explicit external ASS/SubRip mux and conversion reference comparisons.
mod ass {
use fvid::container::{
    matroska_write::{Encoding, PacketWriter, TrackSpec},
};
use std::io::Cursor;
const HEADER: &[u8] = b"[Script Info]\nScriptType: v4.00+\nPlayResX: 384\nPlayResY: 288\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,16,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n";
fn encoded() -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new(
        &mut output,
        &[TrackSpec {
            encoding: Encoding::Ass {
                configuration: HEADER,
            },
            name: "Captions",
            language: "eng",
        }],
    )
    .unwrap();
    writer
        .write_packet(
            0,
            100_000_000,
            900_000_000,
            true,
            b"0,0,Default,,0,0,0,,Hello\\Nworld",
        )
        .unwrap();
    writer
        .write_packet(
            0,
            2_000_000_000,
            500_000_000,
            true,
            "1,0,Default,,0,0,0,,Привет, мир".as_bytes(),
        )
        .unwrap();
    assert_eq!(writer.finish().unwrap().packets, 2);
    output.into_inner()
}
fn independent_decoder_reads_owned_ass_track() {
    let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let path = std::env::temp_dir().join(format!("fvid-ass-{}.mkv", std::process::id()));
    std::fs::write(&path, encoded()).unwrap();
    let result = std::process::Command::new(binary)
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args(["-map", "0:s:0", "-f", "srt", "-"])
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("00:00:00,100 --> 00:00:01,000"), "{text}");
    assert!(text.contains("00:00:02,000 --> 00:00:02,500"), "{text}");
    assert!(text.contains("Hello\nworld"), "{text}");
    assert!(text.contains("Привет, мир"), "{text}");
}

pub fn run() {
    independent_decoder_reads_owned_ass_track();
}
}

mod convert {
use std::path::PathBuf;
const SRT: &str = "1\r\n00:00:00,123 --> 00:00:01,234\r\n<b>Hello</b>\r\nworld\r\n\r\n2\r\n00:00:00,500 --> 00:00:02,000\r\nПривет, мир &amp; &#x41;\r\n";
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


pub fn run() {
    independent_decoder_preserves_srt_cue_times_text_and_style();
    matroska_subrip_selection_uses_owned_conversion_and_preserves_cues();
    font_attributes_match_independent_subrip_conversion();
}
}

fn main() {
    std::env::var("FVID_REFERENCE_FFMPEG").expect("set FVID_REFERENCE_FFMPEG");
    ass::run();
    convert::run();
    println!("ASS mux, SRT cues, SubRip selection and font reference suites passed");
}
