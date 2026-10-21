use fvid_opus::OggOpusReader;
use std::{io::Cursor, path::PathBuf};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/ogg-continuation-boundary")
            .join(name),
    )
    .unwrap()
}
fn read(name: &str) -> Vec<Vec<u8>> {
    let mut reader = OggOpusReader::new(Cursor::new(bytes(name))).unwrap();
    let mut out = Vec::new();
    while let Some(p) = reader.read_packet().unwrap() {
        out.push(p.data);
    }
    out
}
#[test]
fn complete_packets_and_nil_termination_keep_exact_payloads() {
    for (name, parts) in [
        ("one-page-padded.ogg", vec!["padded20.packet"]),
        ("split-padded.ogg", vec!["padded20.packet"]),
        ("completed-prefix.ogg", vec!["padded20.packet"]),
        ("zero-terminator.ogg", vec!["exact255.packet"]),
        (
            "parent-one-page.ogg",
            vec!["parent40.packet", "raw20.packet"],
        ),
        (
            "parent-three-pages.ogg",
            vec!["parent40.packet", "raw20.packet"],
        ),
        ("plain-ten.ogg", vec!["raw10.packet"]),
    ] {
        assert_eq!(
            read(name),
            parts.iter().map(|n| bytes(n)).collect::<Vec<_>>(),
            "{name}"
        );
    }
}
#[test]
fn malformed_missing_continuation_remains_an_explicit_refusal() {
    for name in [
        "missing-final-prefix.ogg",
        "missing-middle.ogg",
        "missing-zero-terminator-prefix.ogg",
    ] {
        let mut reader = OggOpusReader::new(Cursor::new(bytes(name))).unwrap();
        let mut error = None;
        loop {
            match reader.read_packet() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => {
                    error = Some(e.to_string());
                    break;
                }
            }
        }
        assert!(error.unwrap().contains("continuation"), "{name}");
    }
    let mut reader = OggOpusReader::new(Cursor::new(bytes("empty-bridge.ogg"))).unwrap();
    let error = reader.read_packet().unwrap_err().to_string();
    assert!(error.contains("empty segment table"));
}
#[test]
fn baseline_joined_prefix_is_refused_instead_of_publishing_its_fragment() {
    for name in [
        "joined-tail.ogg",
        "joined-two-pages.ogg",
        "joined-zero-terminator.ogg",
    ] {
        let mut reader = OggOpusReader::new(Cursor::new(bytes(name))).unwrap();
        let error = reader.read_packet().unwrap_err().to_string();
        assert!(error.contains("continuation"), "{name}:{error}");
    }
}
#[test]
#[ignore = "enable with joined-prefix fragment handling"]
fn joined_prefix_discards_every_fragment_then_rewind_repeats_the_whole_audio() {
    for name in [
        "joined-tail.ogg",
        "joined-two-pages.ogg",
        "joined-zero-terminator.ogg",
    ] {
        let mut reader = OggOpusReader::new(Cursor::new(bytes(name))).unwrap();
        for pass in 0..3 {
            if pass > 0 {
                reader.rewind().unwrap();
            }
            let packet = reader.read_packet().unwrap().unwrap();
            assert_eq!(packet.data, bytes("raw20.packet"), "{name}:pass{pass}");
            assert_eq!(packet.page_granule, 2880);
            assert!(packet.end_of_stream);
            assert!(reader.read_packet().unwrap().is_none());
        }
    }
}
