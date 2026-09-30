use fvid::container::{
    matroska_write::{Encoding, PacketWriter, TrackSpec},
    webm::WebmReader,
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
#[test]
fn ass_track_preserves_header_events_and_times() {
    let bytes = encoded();
    let mut reader = WebmReader::open(Cursor::new(bytes), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks[0].kind, 17);
    assert_eq!(reader.tracks[0].codec, "S_TEXT/ASS");
    assert_eq!(reader.tracks[0].codec_private, HEADER);
    assert_eq!(reader.packets.len(), 2);
    assert_eq!(
        reader.read_packet(0).unwrap(),
        b"0,0,Default,,0,0,0,,Hello\\Nworld"
    );
}
#[test]
fn invalid_ass_header_fails_before_writing() {
    for header in [&b""[..], b"[Script Info]\nScriptType: v4.00+", b"\xff"] {
        let mut output = Cursor::new(Vec::new());
        assert!(
            PacketWriter::new(
                &mut output,
                &[TrackSpec {
                    encoding: Encoding::Ass {
                        configuration: header
                    },
                    name: "",
                    language: ""
                }]
            )
            .is_err()
        );
        assert!(output.into_inner().is_empty());
    }
}
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
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
