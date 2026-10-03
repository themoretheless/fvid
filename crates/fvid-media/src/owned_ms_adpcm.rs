//! FVid reconstruction of mono/stereo Microsoft ADPCM blocks.
#![forbid(unsafe_code)]
pub use crate::owned_ima4::Error;
type Result<T> = std::result::Result<T, Error>;
const ADAPT: [i64; 16] = [
    230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
];
pub struct MsAdpcmDecoder {
    rate: u32,
    channels: u16,
    block: usize,
    frames: usize,
    coefficients: Vec<(i16, i16)>,
}
impl MsAdpcmDecoder {
    pub fn new(config: &[u8], rate: u32, channels: u16) -> Result<Self> {
        if config.len() < 22 || rate == 0 || !(1..=2).contains(&channels) {
            return Err(Error(
                "MS ADPCM requires WAVEFORMATEX with coefficients and mono/stereo geometry".into(),
            ));
        }
        let word = |at| u16::from_le_bytes([config[at], config[at + 1]]);
        let block = usize::from(word(12));
        let count = usize::from(word(20));
        let extension = usize::from(word(16));
        if word(0) != 2
            || word(2) != channels
            || word(14) != 4
            || u32::from_le_bytes(config[4..8].try_into().unwrap()) != rate
        {
            return Err(Error(
                "MS ADPCM configuration disagrees with codec/track geometry".into(),
            ));
        }
        if block < 7 * usize::from(channels)
            || !(1..=256).contains(&count)
            || extension < 4 + count * 4
            || config.len() < 18 + extension
        {
            return Err(Error(
                "MS ADPCM block or coefficient table is incomplete".into(),
            ));
        }
        let frames = 2 + (block - 7 * usize::from(channels)) * 2 / usize::from(channels);
        if usize::from(word(18)) != frames {
            return Err(Error(
                "MS ADPCM samples-per-block disagrees with block size".into(),
            ));
        }
        let coefficients = (0..count)
            .map(|i| (word(22 + i * 4) as i16, word(24 + i * 4) as i16))
            .collect();
        Ok(Self {
            rate,
            channels,
            block,
            frames,
            coefficients,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    pub fn decode_pcm(&self, packet: &[u8]) -> Result<Vec<f32>> {
        if packet.is_empty() || !packet.len().is_multiple_of(self.block) {
            return Err(Error(
                "MS ADPCM packet does not contain complete blocks".into(),
            ));
        }
        let channels = usize::from(self.channels);
        let count = (packet.len() / self.block)
            .checked_mul(self.frames * channels)
            .ok_or_else(|| Error("MS ADPCM sample count overflow".into()))?;
        let mut out = Vec::new();
        out.try_reserve_exact(count)
            .map_err(|_| Error("MS ADPCM output allocation failed".into()))?;
        out.resize(count, 0.0);
        for (number, block) in packet.chunks_exact(self.block).enumerate() {
            let mut states = [State::default(); 2];
            let origin = number * self.frames * channels;
            let signed = |at| i64::from(i16::from_le_bytes([block[at], block[at + 1]]));
            for ch in 0..channels {
                let &(a, b) = self
                    .coefficients
                    .get(usize::from(block[ch]))
                    .ok_or_else(|| {
                        Error("MS ADPCM predictor index exceeds coefficient table".into())
                    })?;
                let state = State {
                    a: i64::from(a),
                    b: i64::from(b),
                    delta: signed(channels + ch * 2),
                    recent: signed(channels * 3 + ch * 2),
                    older: signed(channels * 5 + ch * 2),
                };
                if state.delta <= 0 {
                    return Err(Error("MS ADPCM initial delta must be positive".into()));
                }
                out[origin + ch] = state.older as f32 / 32768.0;
                out[origin + channels + ch] = state.recent as f32 / 32768.0;
                states[ch] = state;
            }
            for (at, &byte) in block[7 * channels..].iter().enumerate() {
                let sample = 2 + at * 2 / channels;
                out[origin + sample * channels] = states[0].expand(byte >> 4);
                let channel = channels - 1;
                out[origin + sample * channels + 1] = states[channel].expand(byte & 15);
            }
        }
        Ok(out)
    }
}
#[derive(Clone, Copy, Default)]
struct State {
    a: i64,
    b: i64,
    delta: i64,
    recent: i64,
    older: i64,
}
impl State {
    fn expand(&mut self, code: u8) -> f32 {
        let signed = if code & 8 == 0 {
            i64::from(code)
        } else {
            i64::from(code) - 16
        };
        let value = ((self.recent * self.a + self.older * self.b) / 256 + signed * self.delta)
            .clamp(-32768, 32767);
        self.older = self.recent;
        self.recent = value;
        self.delta =
            ((self.delta * ADAPT[usize::from(code)]) >> 8).clamp(16, i64::from(i32::MAX) / 768);
        value as f32 / 32768.0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_blocks_restart_and_reject_incomplete_geometry() {
        let bytes =
            include_bytes!("../../../tests/fixtures/playback-errors/ms-adpcm-stereo-edits.mov");
        let mut reader =
            crate::owned_mp4::Mp4Reader::open(std::io::Cursor::new(bytes), Default::default())
                .unwrap();
        let track = reader.tracks()[0].clone();
        let decoder =
            MsAdpcmDecoder::new(&track.configuration, track.sample_rate, track.channels).unwrap();
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        let expected: Vec<f32> = [-16.0, 0.0, 16.0, 32.0, 48.0, 64.0]
            .into_iter()
            .flat_map(|x| [x / 32768.0, -128.0 / 32768.0])
            .collect();
        assert_eq!(decoder.decode_pcm(&packet).unwrap(), expected);
        packet.extend_from_within(..);
        let decoded = decoder.decode_pcm(&packet).unwrap();
        assert_eq!(&decoded[..12], &decoded[12..]);
        assert!(decoder.decode_pcm(&packet[..packet.len() - 1]).is_err());
        packet[0] = 7;
        assert!(
            decoder
                .decode_pcm(&packet)
                .unwrap_err()
                .to_string()
                .contains("predictor index")
        );
        assert!(MsAdpcmDecoder::new(&track.configuration[..22], 48000, 2).is_err());
        let mut wrong = track.configuration.clone();
        wrong[18] = 7;
        assert!(MsAdpcmDecoder::new(&wrong, 48000, 2).is_err());
    }
    #[test]
    fn signed_prediction_truncation_saturation_and_delta_adaptation() {
        let mut s = State {
            a: 192,
            b: 64,
            recent: -3,
            older: 0,
            delta: 16,
        };
        assert_eq!(s.expand(0), -2.0 / 32768.0);
        assert_eq!(s.expand(15), -18.0 / 32768.0);
        assert_eq!(s.delta, 16);
        let mut s = State {
            a: 256,
            b: 0,
            recent: 32760,
            older: 0,
            delta: 32767,
        };
        assert_eq!(s.expand(7), 32767.0 / 32768.0);
        assert_eq!(s.delta, 78589);
        for _ in 0..100 {
            s.expand(8);
        }
        assert!(s.delta <= i64::from(i32::MAX) / 768);
    }
}
