//! FVid-owned Macintosh Audio Compression/Expansion, shared with playback.
#![forbid(unsafe_code)]
#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_mace_impl.rs");

#[cfg(test)]
mod tests {
    use super::{MaceCoding, MaceDecoder};
    fn coded(aiff: &[u8]) -> &[u8] {
        let mut at = 12;
        while at + 8 <= aiff.len() {
            let n = u32::from_be_bytes(aiff[at + 4..at + 8].try_into().unwrap()) as usize;
            if &aiff[at..at + 4] == b"SSND" {
                return &aiff[at + 16..at + 8 + n];
            }
            at += 8 + n + (n & 1);
        }
        panic!("authored codebook must have SSND");
    }
    #[test]
    fn codebook_pcm_and_packet_boundaries_match_existing_static_references() {
        for (coding, encoded, reference) in [
            (
                MaceCoding::ThreeToOne,
                include_bytes!("../../../tests/fixtures/mace/mac3-codebook.aiff").as_slice(),
                include_bytes!("../../../tests/fixtures/mace/mac3-codebook.s16").as_slice(),
            ),
            (
                MaceCoding::SixToOne,
                include_bytes!("../../../tests/fixtures/mace/mac6-codebook.aiff").as_slice(),
                include_bytes!("../../../tests/fixtures/mace/mac6-codebook.s16").as_slice(),
            ),
        ] {
            let data = coded(encoded);
            let expected: Vec<f32> = reference
                .chunks_exact(2)
                .map(|v| f32::from(i16::from_le_bytes(v.try_into().unwrap())) / 32768.)
                .collect();
            let mut decoder = MaceDecoder::new(coding, 22050, 1).unwrap();
            assert_eq!(decoder.decode_pcm(data).unwrap(), expected);
            decoder.reset();
            let mut split = Vec::new();
            for chunk in data.chunks(coding.block_bytes(1) * 7) {
                split.extend(decoder.decode_pcm(chunk).unwrap());
            }
            assert_eq!(split, expected);
        }
    }
}
