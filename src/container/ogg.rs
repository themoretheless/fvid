//! Compatibility surface for the library-owned Ogg parser.
#[cfg(all(test, feature = "player"))]
use fvid_media::owned_ogg::checksum;
pub use fvid_media::owned_ogg::{Chain, Coding, Limits, Ogg, Packet};

/// Re-seal every page of a file whose bytes a test has already changed, so that
/// the refusal it proves is the one about the field it broke rather than the one
/// about the checksum. A doctored copy that trips the CRC first has proved only
/// that the CRC fires.
#[cfg(all(test, feature = "player"))]
pub(crate) fn rechecksum(bytes: &mut [u8]) {
    let mut at = 0usize;
    while bytes.get(at..at + 4) == Some(b"OggS") {
        let Some(head) = bytes.get(at..at + 27) else {
            return;
        };
        let counted = usize::from(head[26]);
        let Some(table) = bytes.get(at + 27..at + 27 + counted) else {
            return;
        };
        let body: usize = table.iter().map(|length| usize::from(*length)).sum();
        let end = at + 27 + counted + body;
        if end > bytes.len() {
            return;
        }
        let sum = checksum(at, end, bytes);
        bytes[at + 22..at + 26].copy_from_slice(&sum.to_le_bytes());
        at = end;
    }
}

#[cfg(all(test, feature = "player"))]
mod checksum_regression {
    #[test]
    fn resealing_last_ogg_page_preserves_the_intended_granule_mutation() {
        let mut bytes =
            include_bytes!("../../tests/fixtures/playback-errors/opus-surround-presentation.opus")
                .to_vec();
        let mut at = 0;
        let mut last = 0;
        while at < bytes.len() {
            last = at;
            let segments = usize::from(bytes[at + 26]);
            let payload: usize = bytes[at + 27..at + 27 + segments]
                .iter()
                .map(|&v| usize::from(v))
                .sum();
            at += 27 + segments + payload;
        }
        assert_eq!(at, bytes.len());
        bytes[last + 6] ^= 1;
        super::rechecksum(&mut bytes);
        let parsed = super::Ogg::parse(&bytes, &Default::default()).unwrap();
        assert_eq!(
            parsed.chains()[0].packets.last().unwrap().end_granule,
            Some(5622)
        );
    }
}
