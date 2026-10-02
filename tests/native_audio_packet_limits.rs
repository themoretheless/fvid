use fvid::container::adts::StreamReader;
use std::io::{Cursor, Read};

#[test]
fn adts_pce_budget_is_checked_before_bootstrap_payload_read() {
    let fixture = include_bytes!("fixtures/audio/aac-pce-wide8.aac");
    let header = fvid::container::adts::header(&fixture[..7]).unwrap();
    assert_eq!(header.channels, 0);
    struct HeaderOnly(Cursor<Vec<u8>>);
    impl Read for HeaderOnly {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            assert!(self.0.position() < 7, "payload read before budget refusal");
            self.0.read(out)
        }
    }
    let result = StreamReader::open_with_packet_limit(
        HeaderOnly(Cursor::new(fixture[..7].to_vec())),
        header.frame_bytes - header.header_bytes - 1,
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("ADTS packet exceeds budget")
    );
    // A sufficient limit really bootstraps the synthetic PCE and reads packets.
    let mut reader = StreamReader::open_with_packet_limit(Cursor::new(fixture), 8191).unwrap();
    assert_eq!(reader.configuration().channels, 8);
    assert!(reader.next_packet().unwrap().is_some());
}

#[test]
fn adts_payload_limit_applies_to_later_frames_and_excludes_framing() {
    fn frame(payload: &[u8]) -> Vec<u8> {
        let size = payload.len() + 7;
        let mut data = vec![
            0xff,
            0xf1,
            0x50,
            0x40 | ((size >> 11) as u8),
            (size >> 3) as u8,
            ((size & 7) as u8) << 5 | 31,
            0xfc,
        ];
        data.extend_from_slice(payload);
        data
    }
    let bytes = [frame(&[0xe0, 0, 0]), frame(&[0xe0, 0, 0, 0])].concat();
    let mut reader = StreamReader::open_with_packet_limit(Cursor::new(bytes), 3).unwrap();
    assert_eq!(reader.next_packet().unwrap().unwrap(), [0xe0, 0, 0]);
    assert!(
        reader
            .next_packet()
            .unwrap_err()
            .to_string()
            .contains("ADTS packet exceeds budget")
    );
    assert!(reader.next_packet().unwrap().is_none());
}
