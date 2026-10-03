//! Owned bounded, incrementally indexed WebM/Matroska reader.
pub use crate::owned_ebml::{Error, Result};
use crate::owned_file_tags::FileTags;
use crate::owned_matroska::{
    Chapter, ColourDescription, ContentLight, HdrMetadata, MasteringDisplay,
};
use std::io::{Read, Seek, SeekFrom};
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_webm_reader_impl.rs");
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn library_indexes_synthetic_video_and_reads_packet() {
        fn atom(id: &[u8], data: &[u8]) -> Vec<u8> {
            assert!(data.len() < 127);
            [id, &[0x80 | data.len() as u8], data].concat()
        }
        let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
        let video = atom(
            &[0xe0],
            &[atom(&[0xb0], &[16]), atom(&[0xba], &[8])].concat(),
        );
        let track = atom(
            &[0xae],
            &[
                atom(&[0xd7], &[1]),
                atom(&[0x83], &[1]),
                atom(&[0x86], b"V_VP9"),
                video,
            ]
            .concat(),
        );
        let tracks = atom(&[0x16, 0x54, 0xae, 0x6b], &track);
        let cluster = atom(
            &[0x1f, 0x43, 0xb6, 0x75],
            &[
                atom(&[0xe7], &[2]),
                atom(&[0xa3], &[0x81, 0, 1, 0x80, 0x82, 0x49]),
            ]
            .concat(),
        );
        let file = [header, vec![0x18, 0x53, 0x80, 0x67, 0xff], tracks, cluster].concat();
        let mut reader = WebmReader::open(Cursor::new(file), Limits::default()).unwrap();
        assert_eq!(reader.tracks[0].codec, "V_VP9");
        assert_eq!(reader.tracks[0].visible(), (16, 8));
        assert_eq!(reader.packets[0].pts_ns, 3_000_000);
        assert_eq!(reader.read_packet(0).unwrap(), [0x82, 0x49]);
    }
}

#[cfg(test)]
mod index_allocation_tests {
    use super::*;
    #[test]
    fn packet_capacity_respects_non_power_of_two_index_limit() {
        let fixture = include_bytes!("../../../tests/fixtures/playback-errors/ffv1-six-frames.mkv");
        let mut reader = WebmReader::open(
            std::io::Cursor::new(fixture),
            Limits {
                packets: 6,
                ..Default::default()
            },
        )
        .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), 6);
        assert_eq!(reader.packets.capacity(), 6);
        assert_eq!(
            reader.packet_index_payload_bytes().unwrap(),
            6 * std::mem::size_of::<Packet>()
        );
        let error = match WebmReader::open(
            std::io::Cursor::new(fixture),
            Limits {
                packets: 5,
                ..Default::default()
            },
        ) {
            Ok(_) => panic!("index limit must refuse the sixth packet"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("WebM packet count exceeds limit")
        );
    }
    #[test]
    fn index_estimate_counts_metadata_and_codec_private_spare_capacity() {
        let fixture = include_bytes!("../../../tests/fixtures/playback-errors/ffv1-six-frames.mkv");
        let mut reader =
            WebmReader::open(std::io::Cursor::new(fixture), Default::default()).unwrap();
        reader.scan_all().unwrap();
        let before = reader.estimated_index_payload_bytes().unwrap();
        assert!(before >= reader.packet_index_payload_bytes().unwrap());
        let mut key = String::with_capacity(64);
        key.push_str("NOTE");
        let mut value = String::with_capacity(128);
        value.push_str("synthetic");
        let metadata_bytes = key.capacity() + value.capacity();
        reader.metadata.insert(key, value);
        assert_eq!(
            reader.estimated_index_payload_bytes().unwrap() - before,
            4096 + metadata_bytes
        );
        let before = reader.estimated_index_payload_bytes().unwrap();
        let old = reader.tracks[0].codec_private.capacity();
        reader.tracks[0].codec_private.reserve(16384);
        let growth = reader.tracks[0].codec_private.capacity() - old;
        assert_eq!(
            reader.estimated_index_payload_bytes().unwrap() - before,
            growth
        );
    }
    #[test]
    fn admitted_scan_preserves_full_scan_metadata_and_timestamps() {
        let fixture =
            include_bytes!("../../../tests/fixtures/playback-errors/ffv1-custom-tags.mkv");
        let mut expected =
            WebmReader::open(std::io::Cursor::new(fixture), Default::default()).unwrap();
        expected.scan_all().unwrap();
        let mut admitted =
            WebmReader::open(std::io::Cursor::new(fixture), Default::default()).unwrap();
        admitted
            .scan_all_with_admission(|reader| {
                reader.estimated_index_payload_bytes()?;
                Ok(())
            })
            .unwrap();
        assert_eq!(admitted.tags, expected.tags);
        assert_eq!(admitted.metadata, expected.metadata);
        assert_eq!(admitted.track_metadata, expected.track_metadata);
        assert_eq!(admitted.chapters, expected.chapters);
        assert_eq!(admitted.duration_ns, expected.duration_ns);
        assert_eq!(
            admitted
                .packets
                .iter()
                .map(|packet| (packet.pts_ns, packet.size))
                .collect::<Vec<_>>(),
            expected
                .packets
                .iter()
                .map(|packet| (packet.pts_ns, packet.size))
                .collect::<Vec<_>>()
        );
    }
}
