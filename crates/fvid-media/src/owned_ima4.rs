//! Apple QuickTime IMA4 packet reconstruction, implemented by FVid.
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
// The standard IMA quantizer ladder and index adaptation values.
const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const ADAPT: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];
pub struct Ima4Decoder {
    rate: u32,
    channels: u16,
}
impl Ima4Decoder {
    pub fn new(rate: u32, channels: u16) -> Result<Self> {
        if rate == 0 || !(1..=64).contains(&channels) {
            return Err(Error(
                "IMA4 requires a sample rate and 1..64 channels".into(),
            ));
        }
        Ok(Self { rate, channels })
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    /// Each group holds one 34-byte block per channel, yielding 64 frames.
    /// Predictor/index are reset from each header; packets share no mutable state.
    pub fn decode_pcm(&self, packet: &[u8]) -> Result<Vec<f32>> {
        let channels = usize::from(self.channels);
        let group = channels * 34;
        if packet.is_empty() || !packet.len().is_multiple_of(group) {
            return Err(Error(
                "IMA4 packet does not contain complete channel blocks".into(),
            ));
        }
        let count = (packet.len() / group)
            .checked_mul(64 * channels)
            .ok_or_else(|| Error("IMA4 sample count overflow".into()))?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(count)
            .map_err(|_| Error("IMA4 output allocation failed".into()))?;
        output.resize(count, 0.0);
        for (group_index, blocks) in packet.chunks_exact(group).enumerate() {
            for (channel, block) in blocks.chunks_exact(34).enumerate() {
                let header = u16::from_be_bytes([block[0], block[1]]);
                let mut predictor = i32::from((header & 0xff80) as i16);
                // QuickTime decoders saturate the seven-bit index to the ladder.
                let mut index = i32::from(header & 127).min(88);
                for sample in 0..64 {
                    let code = (block[2 + sample / 2] >> ((sample % 2) * 4)) & 15;
                    output[(group_index * 64 + sample) * channels + channel] =
                        expand_sample(&mut predictor, &mut index, code);
                }
            }
        }
        Ok(output)
    }
}
pub(crate) fn expand_sample(predictor: &mut i32, index: &mut i32, code: u8) -> f32 {
    // IMA rounds each contribution before summing; multiplying first changes PCM.
    let step = STEPS[*index as usize];
    let mut change = step >> 3;
    if code & 1 != 0 {
        change += step >> 2;
    }
    if code & 2 != 0 {
        change += step >> 1;
    }
    if code & 4 != 0 {
        change += step;
    }
    *predictor = (*predictor + if code & 8 == 0 { change } else { -change }).clamp(-32768, 32767);
    *index = (*index + ADAPT[(code & 7) as usize]).clamp(0, 88);
    *predictor as f32 / 32768.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_step_delta_rounds_each_term_before_summing() {
        let mut predictor = 0;
        let mut index = 0;
        assert_eq!(expand_sample(&mut predictor, &mut index, 1), 1.0 / 32768.0);
        assert_eq!(expand_sample(&mut predictor, &mut index, 9), 0.0);
        assert_eq!(index, 0);
    }
    #[test]
    fn block_headers_nibble_order_channels_and_restart_have_exact_samples() {
        let mut packet = vec![0, 0];
        packet.extend([0x11; 32]);
        packet.extend([0xff, 0x80]);
        packet.extend([0; 32]);
        let decoder = Ima4Decoder::new(48000, 2).unwrap();
        let actual = decoder.decode_pcm(&packet).unwrap();
        for frame in 0..64 {
            assert_eq!(
                &actual[frame * 2..frame * 2 + 2],
                &[(frame + 1) as f32 / 32768.0, -128.0 / 32768.0]
            );
        }
        packet.extend_from_within(..);
        let twice = decoder.decode_pcm(&packet).unwrap();
        assert_eq!(&twice[..128], &twice[128..]);
        assert!(decoder.decode_pcm(&packet[..packet.len() - 1]).is_err());
        let mut alternating = vec![0, 0];
        alternating.extend([0x81; 32]);
        let mono = Ima4Decoder::new(48000, 1).unwrap();
        let samples = mono.decode_pcm(&alternating).unwrap();
        assert_eq!(
            &samples[..4],
            &[1.0 / 32768.0, 1.0 / 32768.0, 2.0 / 32768.0, 2.0 / 32768.0]
        );
        let mut saturated = vec![0, 127];
        saturated.extend([0x77; 32]);
        assert!(
            mono.decode_pcm(&saturated)
                .unwrap()
                .iter()
                .all(|&x| x == 32767.0 / 32768.0)
        );
    }
}
