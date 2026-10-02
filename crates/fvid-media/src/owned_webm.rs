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
