use fvid::codec::{av1::Obus, av1_decoder::Decoder};
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("av1-seg-inherit-generated.json")).unwrap())
        .unwrap()
}
#[test]
fn inherited_zero_map_segmentation_decodes_all_reference_slots_and_roles() {
    let manifest = manifest();
    let fixtures = manifest["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 224);
    for record in fixtures {
        let name = record["file"].as_str().unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        let mut decoder = Decoder::new(8 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 7, "{name}");
            assert!(!frames[0].show);
            assert_eq!(frames.iter().filter(|f| f.show).count(), 6);
            for frame in frames {
                assert_eq!(frame.picture.size, [32, 32]);
                assert!(
                    frame
                        .picture
                        .planes
                        .iter()
                        .all(|p| p.samples.iter().all(|&v| v == 128)),
                    "{name}"
                );
            }
            decoder.reset();
        }
    }
}
#[test]
fn remaining_segmentation_tools_and_truncated_data_have_specific_refusals() {
    for record in manifest()["refusals"].as_array().unwrap() {
        assert_eq!(record["acceptance"], false);
        let name = record["file"].as_str().unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        let mut packets = Vec::new();
        let mut offset = 0;
        for obu in Obus::new(&bytes) {
            let obu = obu.unwrap();
            let end = obu.payload.as_ptr() as usize - bytes.as_ptr() as usize + obu.payload.len();
            packets.push(&bytes[offset..end]);
            offset = end;
        }
        let mut decoder = Decoder::new(8 << 20);
        for _ in 0..2 {
            for packet in &packets[..packets.len() - 1] {
                for frame in decoder.decode_packet(packet).unwrap() {
                    assert!(!frame.show);
                    assert!(
                        frame
                            .picture
                            .planes
                            .iter()
                            .all(|p| p.samples.iter().all(|&v| v == 128))
                    );
                }
            }
            let packet = packets.last().unwrap();
            let error = decoder.decode_packet(packet).err().unwrap();
            assert!(
                error
                    .to_string()
                    .contains(record["error"].as_str().unwrap()),
                "{name}: {error}"
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
fn inherited_segmentation_webm_preserves_display_timing_rewind_and_seek() {
    let mut cases = 0;
    for record in manifest()["fixtures"].as_array().unwrap() {
        let Some(name) = record["webm"].as_str() else {
            continue;
        };
        let bytes = std::fs::read(root().join(name)).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(&bytes), 8 << 20).unwrap();
        for _ in 0..2 {
            for index in 0..6u128 {
                let planes = reader
                    .read_frame_planes()
                    .unwrap_or_else(|e| panic!("{name}: {e}"))
                    .unwrap();
                assert!(
                    planes
                        .y
                        .iter()
                        .chain(&planes.cb)
                        .chain(&planes.cr)
                        .all(|&v| v == 128)
                );
                assert_eq!(
                    reader.frame_interval(),
                    Some((index * 20_000_000, (index + 1) * 20_000_000, 1_000_000_000)),
                    "{name}"
                );
            }
            assert!(reader.read_frame_planes().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
        assert!(reader.read_frame_planes().unwrap().is_some());
        cases += 1;
    }
    assert_eq!(cases, 16);
}
