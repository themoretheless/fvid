//! Uncompressed PCM. The container states the sample width and byte order, so
//! decoding is a widening conversion into the interleaved f32 the pipeline
//! carries - no codec state, nothing to reset.
//!
//! Both containers name the same thing differently: ISO BMFF uses one fourcc per
//! byte order (`sowt` little, `twos` big) with the width in the sample entry,
//! while Matroska has `A_PCM/INT/LIT`, `A_PCM/INT/BIG` and `A_PCM/FLOAT/IEEE`
//! with the width in `BitDepth`.

use crate::audio::AudioPacket;
use crate::invalid;

pub use fvid_media::owned_pcm_decoder::{PcmDecoder, PcmFormat};

impl crate::audio::AudioDecode for PcmDecoder {
    /// Nothing is decoded: the packet's timestamp is the caller's and its bytes
    /// become output in the same order, so a packet always yields a packet.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        if matches!(self.format(), PcmFormat::ALaw | PcmFormat::MuLaw)
            && !data.len().is_multiple_of(usize::from(self.channels()))
        {
            return Err(invalid("G.711 packet ends in an incomplete channel frame"));
        }
        Ok(Some(AudioPacket {
            data: self.convert(data)?,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate(),
        }))
    }

    fn reset(&mut self) {}
}

#[cfg(all(test, feature = "player"))]
mod tests {
    #[test]
    fn every_container_name_reaches_its_own_layout() {
        for (codec, bits, bytes, expected) in [
            ("A_PCM/INT/LIT", 8u16, &[0x40][..], -0.5),
            ("A_PCM/INT/BIG", 8, &[0xc0], 0.5),
            ("sowt", 16u16, &[0x00, 0x80][..], -1.0),
            ("A_PCM/INT/LIT", 16, &[0x00, 0x80], -1.0),
            ("twos", 16, &[0x80, 0x00], -1.0),
            ("A_PCM/INT/BIG", 24, &[0x80, 0x00, 0x00], -1.0),
            ("A_PCM/INT/LIT", 0, &[0xff, 0x7f], 32_767.0 / 32_768.0),
            ("fl32", 16, &[0x00, 0x00, 0x00, 0xbf], -0.5),
            (
                "A_PCM/FLOAT/IEEE",
                64,
                &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xd0, 0x3f],
                0.25,
            ),
            (
                "fl64",
                0,
                &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xd0, 0x3f],
                0.25,
            ),
        ] {
            let mut decoder = crate::codec::make_audio_decoder(codec, &[], 48_000, 1, bits)
                .unwrap_or_else(|error| panic!("{codec}: {error}"));
            let packet = decoder
                .decode_encoded(bytes, 0, 0)
                .expect("convert")
                .expect("PCM always yields a packet");
            let samples: Vec<f32> = packet
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| f32::from_le_bytes(*chunk))
                .collect();
            assert_eq!(samples, vec![expected], "{codec} at {bits} bits");
        }
    }
}
