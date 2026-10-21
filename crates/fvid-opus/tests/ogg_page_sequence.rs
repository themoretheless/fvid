use fvid_opus::{OggOpusReader, OggPacket};
use std::{io::Cursor, path::PathBuf};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/ogg-page-sequence")
            .join(name),
    )
    .unwrap()
}
#[test]
fn monostream_packets_and_rewind_preserve_origins_and_wrapped_counters() {
    for (name, count, origin) in [
        ("complete-control.ogg", 3, 0),
        ("positive-origin-control.ogg", 2, 960),
        ("legacy-counter-control.ogg", 3, 0),
        ("wrapped-counter-control.ogg", 3, 0),
    ] {
        let mut reader = OggOpusReader::new(Cursor::new(bytes(name))).unwrap();
        for pass in 0..3 {
            if pass > 0 {
                reader.rewind().unwrap();
            }
            let mut packets = Vec::new();
            if pass == 1 {
                while let Some(packet) = reader.read_packet().unwrap() {
                    packets.push(packet);
                }
            } else {
                let mut packet = OggPacket::default();
                while reader.read_packet_into(&mut packet).unwrap() {
                    packets.push(packet.clone());
                }
            }
            assert_eq!(packets.len(), count, "{name}:pass{pass}");
            for (index, packet) in packets.iter().enumerate() {
                assert_eq!(packet.data, [0xf8, 0xff, 0xfe]);
                assert_eq!(packet.page_granule, origin + (index as i64 + 1) * 960);
                assert_eq!(packet.end_of_stream, index + 1 == count);
            }
        }
    }
}
#[test]
fn streaming_reader_reports_discontinuity_without_publishing_the_bad_page() {
    for (name, prefix) in [
        ("lost-first-audio.ogg", 0),
        ("gap.ogg", 0),
        ("duplicate.ogg", 1),
        ("backward.ogg", 1),
    ] {
        let mut reader = OggOpusReader::new(Cursor::new(bytes(name))).unwrap();
        for pass in 0..2 {
            if pass > 0 {
                reader.rewind().unwrap();
            }
            let mut packet = OggPacket::new(vec![123], -5, false);
            let mut seen = 0;
            loop {
                let previous = packet.clone();
                match reader.read_packet_into(&mut packet) {
                    Ok(true) => {
                        seen += 1;
                        assert!(seen <= prefix, "{name}: bad page was published");
                    }
                    Ok(false) => panic!("{name}: counter contradiction silently reached EOF"),
                    Err(e) => {
                        assert!(e.to_string().contains("sequence"), "{name}:{e}");
                        assert_eq!(seen, prefix);
                        assert_eq!(packet, previous);
                        break;
                    }
                }
            }
        }
    }
}
