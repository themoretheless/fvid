//! Apple MACE: three bits of code to a sample, or to two, with nothing in the byte
//! saying which.

use crate::audio::{AudioDecode, AudioPacket};
use crate::invalid;

pub use fvid_media::owned_mace::{MaceCoding, MaceDecoder, SAMPLES_PER_BLOCK};

impl AudioDecode for MaceDecoder {
    /// Every block yields six samples per channel, so a packet comes back a packet and
    /// the timestamps stay the caller's.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        let planar = self.decode_planar(data)?;
        let frames = planar[0].len();
        let channels = usize::from(self.channels());
        let byte_len = frames
            .checked_mul(channels)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| invalid("MACE packet size overflow"))?;
        let mut out = Vec::new();
        out.try_reserve_exact(byte_len)
            .map_err(|_| invalid("MACE output buffer allocation failed"))?;
        for frame in 0..frames {
            for channel in &planar[..channels] {
                let sample = f32::from(channel[frame]) / 32_768.0;
                out.extend_from_slice(&sample.to_le_bytes());
            }
        }
        Ok(Some(AudioPacket {
            data: out,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate(),
        }))
    }

    /// Both predictors start again at the quiet end of the level table, as a decoder
    /// opened on the stream does.
    fn reset(&mut self) {
        MaceDecoder::reset(self);
    }
}

#[cfg(all(test, feature = "player"))]
mod tests {
    use super::{MaceCoding, MaceDecoder, SAMPLES_PER_BLOCK};
    use crate::audio::{AudioDecode, AudioStream};
    use crate::container::mp4::Limits as Mp4Limits;
    use crate::playback_mp4_audio::Mp4AudioReader;
    use std::io::Cursor;

    const MJPEGA: &[u8] = include_bytes!("../../tests/fixtures/mace/mjpega.mov");
    const MJPEGA_S16: &[u8] = include_bytes!("../../tests/fixtures/mace/mjpega.s16");

    fn reference(bytes: &[u8]) -> Vec<i16> {
        bytes
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes(chunk.try_into().expect("two bytes")))
            .collect()
    }

    fn decode(bytes: &[u8], coding: MaceCoding, channels: u16) -> Vec<i16> {
        let mut decoder = MaceDecoder::new(coding, 22_050, channels).expect("a coding at a rate");
        let packet = decoder
            .decode_encoded(bytes, 0, 0)
            .expect("decodes")
            .expect("a whole block always yields samples");
        assert_eq!((packet.timebase_num, packet.timebase_den), (1, 22_050));
        packet
            .data
            .chunks_exact(4)
            .map(|chunk| {
                (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0).round()
                    as i16
            })
            .collect()
    }

    #[test]
    fn a_mac_take_is_indexed_in_blocks_and_read_one_a_packet() {
        let mut reader = Mp4AudioReader::open(Cursor::new(MJPEGA), Mp4Limits::default())
            .expect("the take opens");
        assert_eq!(reader.codec(), "mace6");
        assert_eq!((reader.sample_rate(), reader.channels()), (8_000, 1));
        let mut heard = Vec::new();
        let mut packets = 0;
        while let Some(packet) = reader.next_packet().expect("packet") {
            assert_eq!(packet.data.len(), 1, "one block a packet");
            assert_eq!(packet.duration, SAMPLES_PER_BLOCK as i64);
            heard.extend_from_slice(&packet.data);
            packets += 1;
        }
        assert_eq!(packets, 2_976);
        assert_eq!(
            decode(&heard, MaceCoding::SixToOne, 1),
            reference(MJPEGA_S16),
            "2 976 single-byte packets must give the reference's 17 856 samples"
        );
    }
}
