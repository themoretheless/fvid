use fvid_opus::{OggOpusReader, OggPacket};
use std::{io::Cursor, path::PathBuf};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/ogg-stream-identity")
            .join(name),
    )
    .unwrap()
}
const BAD: [(&str, u32, u32); 5] = [
    ("missing-bos-zero.ogg", 0, 0),
    ("repeated-tags-bos.ogg", 0x46564920, 0x46564920),
    ("foreign-tags-bos.ogg", 0x46564920, 0x46564921),
    ("repeated-audio-bos.ogg", 0x46564920, 0x46564920),
    ("foreign-audio-bos.ogg", 0x46564920, 0x46564921),
];
fn decoded(reader: &mut OggOpusReader<Cursor<Vec<u8>>>, into: bool) -> usize {
    let mut decoder = reader.head().decoder(48000).unwrap();
    let mut frames = 0;
    let mut packet = OggPacket::default();
    loop {
        let more = if into {
            reader.read_packet_into(&mut packet).unwrap()
        } else {
            match reader.read_packet().unwrap() {
                Some(p) => {
                    packet = p;
                    true
                }
                None => false,
            }
        };
        if !more {
            break;
        }
        assert_eq!(packet.data, bytes("20ms.packet"));
        let mut pcm = vec![0.; 5760];
        let n = decoder.decode(&packet.data, 5760, &mut pcm).unwrap();
        assert_eq!(n, 960);
        assert!(pcm[..n].iter().all(|v| v.is_finite()));
        frames += n;
    }
    frames
}
#[test]
fn healthy_stream_identity_and_zero_serial_survive_both_packet_apis_and_rewind() {
    for (name, serial) in [("normal.ogg", 0x46564920), ("zero-serial.ogg", 0)] {
        let mut reader = OggOpusReader::new(Cursor::new(bytes(name))).unwrap();
        for pass in 0..3 {
            if pass > 0 {
                reader.rewind().unwrap();
            }
            assert_eq!(decoded(&mut reader, pass != 1), 1920);
            assert_eq!(reader.serial(), serial);
            assert_eq!(reader.head().channel_count, 1);
        }
    }
}
#[test]
fn missing_or_repeated_bos_is_refused_before_publishing_or_replacing_identity() {
    for (name, serial, _) in BAD {
        let mut reader = match OggOpusReader::new(Cursor::new(bytes(name))) {
            Ok(r) => r,
            Err(e) => {
                assert!(e.to_string().contains("BOS"), "{name}:{e}");
                continue;
            }
        };
        for pass in 0..2 {
            if pass > 0 {
                reader.rewind().unwrap();
            }
            let mut packet = OggPacket::new(vec![42], -7, false);
            let previous = packet.clone();
            let error = reader.read_packet_into(&mut packet).expect_err(name);
            assert!(error.to_string().contains("BOS"), "{name}:{error}");
            assert_eq!(packet, previous);
            assert_eq!(reader.serial(), serial);
            assert_eq!(reader.head().channel_count, 1);
        }
    }
}
