//! Owned FFV1 v0/v1 decoding, persistent context models and bounded storage.
//! RFC 9043 bitstream core is shared with native frontend playback.
use crate::owned_ffv1_encoder::TRANSITION;
use crate::owned_frame::{GeometryFrame, buffer};
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
fn unsupported(message: &str) -> String {
    message.into()
}
include!("owned_ffv1_decoder_impl.rs");

#[cfg(test)]
mod gray_tests {
    use super::*;
    #[test]
    fn gray_fixtures_preserve_luma_and_expand_neutral_chroma() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 10, 16] {
            let mut decoder = Decoder::new(4, 3, 1 << 20).unwrap();
            for index in 0..2 {
                let packet =
                    std::fs::read(root.join(format!("ffv1-gray-{depth}-{index}.packet"))).unwrap();
                let samples =
                    std::fs::read(root.join(format!("ffv1-gray-{depth}-{index}.gray"))).unwrap();
                let decoded = decoder.decode(&packet).unwrap();
                assert_eq!(decoded.depth, depth);
                assert!(decoded.keyframe);
                assert_eq!(decoded.frame.subsampling, Some([1, 1]));
                assert_eq!(&decoded.frame.data[..samples.len()], samples);
                let bytes = if depth == 8 { 1 } else { 2 };
                let neutral = (1u16 << (depth - 1)).to_le_bytes();
                assert_eq!(decoded.frame.data.len(), samples.len() * 3);
                for pixel in decoded.frame.data[samples.len()..].chunks_exact(bytes) {
                    assert_eq!(pixel, &neutral[..bytes]);
                }
                assert_eq!(
                    crate::owned_ffv1_encoder::encode_gray(4, 3, &samples, depth).unwrap(),
                    packet
                );
                assert!(Decoder::new(4, 3, 1).unwrap().decode(&packet).is_err());
            }
        }
    }
    #[test]
    fn gray_encoding_validates_storage_depth_and_geometry() {
        use crate::owned_ffv1_encoder::encode_gray;
        assert!(encode_gray(0, 3, &[], 8).is_err());
        assert!(encode_gray(4, 3, &[0; 11], 8).is_err());
        assert!(encode_gray(4, 3, &[0; 12], 7).is_err());
        assert!(encode_gray(usize::MAX, 2, &[], 8).is_err());
        assert!(encode_gray(1, 1, &[0xff, 0xff], 10).is_err());
    }
}
