//! Owned Microsoft-spelled IMA ADPCM block reconstruction.
#![forbid(unsafe_code)]
pub use crate::owned_ima4::Error;
type Result<T> = std::result::Result<T, Error>;
pub struct ImaWavDecoder {
    rate: u32,
    channels: u16,
    block: usize,
    frames: usize,
}
impl ImaWavDecoder {
    pub fn new(config: &[u8], rate: u32, channels: u16) -> Result<Self> {
        if config.len() < 16 || rate == 0 || !(1..=2).contains(&channels) {
            return Err(Error(
                "IMA WAV requires WAVEFORMATEX, a sample rate and mono/stereo geometry".into(),
            ));
        }
        let word = |at| u16::from_le_bytes([config[at], config[at + 1]]);
        if word(0) != 17
            || word(2) != channels
            || u32::from_le_bytes(config[4..8].try_into().unwrap()) != rate
            || word(14) != 4
        {
            return Err(Error(
                "IMA WAV configuration disagrees with codec/track geometry".into(),
            ));
        }
        let block = usize::from(word(12));
        let header = usize::from(channels) * 4;
        if block < header || (channels == 2 && !(block - header).is_multiple_of(8)) {
            return Err(Error(
                "IMA WAV block has invalid channel interleaving geometry".into(),
            ));
        }
        let frames = 1 + (block - header) * 2 / usize::from(channels);
        if config.len() >= 20 && usize::from(word(18)) != frames {
            return Err(Error(
                "IMA WAV samples-per-block disagrees with block size".into(),
            ));
        }
        Ok(Self {
            rate,
            channels,
            block,
            frames,
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
                "IMA WAV packet does not contain complete blocks".into(),
            ));
        }
        let channels = usize::from(self.channels);
        let count = (packet.len() / self.block)
            .checked_mul(self.frames * channels)
            .ok_or_else(|| Error("IMA WAV sample count overflow".into()))?;
        let mut out = Vec::new();
        out.try_reserve_exact(count)
            .map_err(|_| Error("IMA WAV output allocation failed".into()))?;
        out.resize(count, 0.0);
        for (number, block) in packet.chunks_exact(self.block).enumerate() {
            let mut states = [(0i32, 0i32); 2];
            let origin = number * self.frames * channels;
            for ch in 0..channels {
                let header = &block[ch * 4..ch * 4 + 4];
                if header[2] > 88 {
                    return Err(Error("IMA WAV step index exceeds 88".into()));
                }
                states[ch] = (
                    i32::from(i16::from_le_bytes([header[0], header[1]])),
                    i32::from(header[2]),
                );
                out[origin + ch] = states[ch].0 as f32 / 32768.0;
            }
            let payload = &block[channels * 4..];
            for (at, &byte) in payload.iter().enumerate() {
                let (ch, sample) = if channels == 1 {
                    (0, 1 + at * 2)
                } else {
                    ((at / 4) % 2, 1 + (at / 8) * 8 + (at % 4) * 2)
                };
                let (predictor, index) = &mut states[ch];
                out[origin + sample * channels + ch] =
                    crate::owned_ima4::expand_sample(predictor, index, byte & 15);
                out[origin + (sample + 1) * channels + ch] =
                    crate::owned_ima4::expand_sample(predictor, index, byte >> 4);
            }
        }
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config(channels: u16, block: u16) -> Vec<u8> {
        let mut c = vec![0; 20];
        for (at, value) in [
            (0, 17),
            (2, channels),
            (12, block),
            (14, 4),
            (16, 2),
            (18, 1 + (block - channels * 4) * 2 / channels),
        ] {
            c[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        c[4..8].copy_from_slice(&48000u32.to_le_bytes());
        c
    }
    #[test]
    fn predictor_headers_stereo_groups_restart_and_refusal_are_exact() {
        let decoder = ImaWavDecoder::new(&config(2, 16), 48000, 2).unwrap();
        let mut packet = vec![32, 0, 0, 7, 128, 255, 0, 0];
        packet.extend([0x11; 4]);
        packet.extend([0; 4]);
        let samples = decoder.decode_pcm(&packet).unwrap();
        for frame in 0..9 {
            assert_eq!(
                &samples[frame * 2..frame * 2 + 2],
                &[(32 + frame) as f32 / 32768.0, -128.0 / 32768.0]
            );
        }
        packet.extend_from_within(..);
        let two = decoder.decode_pcm(&packet).unwrap();
        assert_eq!(&two[..18], &two[18..]);
        assert!(decoder.decode_pcm(&packet[..31]).is_err());
        packet[2] = 89;
        assert!(decoder.decode_pcm(&packet).is_err());
        assert!(ImaWavDecoder::new(&config(2, 14), 48000, 2).is_err());
        let mono = ImaWavDecoder::new(&config(1, 7), 48000, 1).unwrap();
        assert_eq!(
            mono.decode_pcm(&[0, 0, 0, 0, 0x11, 0x11, 0x11]).unwrap(),
            (0..=6).map(|n| n as f32 / 32768.0).collect::<Vec<_>>()
        );
    }
}
