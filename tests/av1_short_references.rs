use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("av1-short-ref-generated.json")).unwrap())
        .unwrap()
}
#[test]
fn short_and_explicit_references_decode_all_order_widths_and_id_combinations() {
    let manifest = manifest();
    let fixtures = manifest["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 298);
    for record in fixtures {
        let name = record["file"].as_str().unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        let expected: [usize; 7] =
            std::array::from_fn(|i| record["references"][i].as_u64().unwrap() as usize);
        let mut decoder = Decoder::new(8 << 20);
        for _ in 0..2 {
            let mut sequence = None;
            let mut headers: [Option<Header>; 8] = std::array::from_fn(|_| None);
            let mut last = None;
            let mut offset = 0;
            let mut shown = 0;
            for obu in Obus::new(&bytes) {
                let obu = obu.unwrap();
                if obu.kind == 1 {
                    sequence = Some(Sequence::parse(obu.payload).unwrap());
                }
                if obu.kind == 6 {
                    let header = Header::parse(
                        sequence.as_ref().unwrap(),
                        obu.payload,
                        0,
                        0,
                        &std::array::from_fn(|i| headers[i].as_ref()),
                    )
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                    for i in 0..8 {
                        if header.refresh_flags & (1 << i) != 0 {
                            headers[i] = Some(header.clone());
                        }
                    }
                    last = Some(header);
                }
                let end =
                    obu.payload.as_ptr() as usize - bytes.as_ptr() as usize + obu.payload.len();
                let frames = decoder
                    .decode_packet(&bytes[offset..end])
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                for frame in frames {
                    assert_eq!(frame.picture.size, [32, 32]);
                    assert!(
                        frame
                            .picture
                            .planes
                            .iter()
                            .all(|p| p.samples.iter().all(|&v| v == 128))
                    );
                    shown += usize::from(frame.show);
                }
                offset = end;
            }
            assert_eq!(shown, 1, "{name}");
            assert_eq!(last.unwrap().references, expected, "{name}");
            decoder.reset();
        }
    }
}
#[test]
fn invalid_short_anchor_has_specific_refusal_and_reset() {
    for record in manifest()["refusals"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(record["file"].as_str().unwrap())).unwrap();
        let mut decoder = Decoder::new(8 << 20);
        for _ in 0..2 {
            let mut packets = Vec::new();
            let mut offset = 0;
            for obu in Obus::new(&bytes) {
                let obu = obu.unwrap();
                let end =
                    obu.payload.as_ptr() as usize - bytes.as_ptr() as usize + obu.payload.len();
                packets.push(&bytes[offset..end]);
                offset = end;
            }
            for packet in &packets[..packets.len() - 1] {
                assert!(
                    decoder
                        .decode_packet(packet)
                        .unwrap()
                        .iter()
                        .all(|f| !f.show)
                );
            }
            let packet = packets.last().unwrap();
            let error = decoder.decode_packet(packet).err().unwrap();
            assert!(
                error
                    .to_string()
                    .contains(record["error"].as_str().unwrap()),
                "{error}"
            );
            assert!(
                decoder
                    .decode_packet(packet)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("requires reset")
            );
            decoder.reset();
        }
    }
}
#[test]
fn short_reference_webm_replays_and_seeks_with_display_clock() {
    for record in manifest()["fixtures"].as_array().unwrap().iter().take(10) {
        let name = record["webm"].as_str().unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(&bytes), 8 << 20).unwrap();
        for _ in 0..2 {
            let frame = reader
                .read_frame_planes()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .unwrap();
            assert!(
                frame
                    .y
                    .iter()
                    .chain(&frame.cb)
                    .chain(&frame.cr)
                    .all(|&v| v == 128)
            );
            assert_eq!(
                reader.frame_interval(),
                Some((0, 20_000_000, 1_000_000_000)),
                "{name}"
            );
            assert!(reader.read_frame_planes().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
        assert!(reader.read_frame_planes().unwrap().is_some());
    }
}
