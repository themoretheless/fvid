//! Owned streaming float PCM gain and standard mono/stereo rematrixing.
use std::io::Write;
// Native decoder writes may split channel frames, but always contain whole samples.
pub struct PcmGain<'a, W> {
    output: &'a mut W,
    gain: f64,
    input_channels: u16,
    output_channels: u16,
    frame: [f64; 6],
    filled: usize,
}
impl<W: Write> Write for PcmGain<'_, W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() % 8 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unaligned PCM write",
            ));
        }
        if self.input_channels == self.output_channels {
            for bytes in data.chunks_exact(8) {
                let value = f64::from_le_bytes(bytes.try_into().unwrap()) * self.gain;
                if !value.is_finite() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "PCM gain overflow",
                    ));
                }
                self.output.write_all(&value.to_le_bytes())?;
                self.filled = (self.filled + 1) % usize::from(self.input_channels);
            }
            return Ok(data.len());
        }
        for bytes in data.chunks_exact(8) {
            self.frame[self.filled] = f64::from_le_bytes(bytes.try_into().unwrap());
            self.filled += 1;
            if self.filled != usize::from(self.input_channels) {
                continue;
            }
            let frame = &self.frame;
            let mut mixed = [0.0; 6];
            // Standard decoded order: FL FR FC [LFE] BL BR or BC.
            // LFE is omitted. Centre/surround contributions use -3 dB.
            let k = std::f64::consts::FRAC_1_SQRT_2;
            let (mut left, mut right) = if self.input_channels == 1 {
                (frame[0], frame[0])
            } else {
                (frame[0], frame[1])
            };
            if self.input_channels >= 3 {
                left += k * frame[2];
                right += k * frame[2];
            }
            match self.input_channels {
                4 => {
                    left += k * frame[3];
                    right += k * frame[3];
                }
                5 => {
                    left += k * frame[3];
                    right += k * frame[4];
                }
                6 => {
                    left += k * frame[4];
                    right += k * frame[5];
                }
                _ => {}
            }
            if self.output_channels == 1 {
                mixed[0] = (left + right) * 0.5;
            } else {
                mixed[0] = left;
                mixed[1] = right;
            }
            for sample in &mixed[..usize::from(self.output_channels)] {
                let value = sample * self.gain;
                if !value.is_finite() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "PCM gain overflow",
                    ));
                }
                self.output.write_all(&value.to_le_bytes())?;
            }
            self.filled = 0;
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

impl<'a, W: Write> PcmGain<'a, W> {
    pub fn new(
        output: &'a mut W,
        gain: f64,
        input_channels: u16,
        output_channels: u16,
    ) -> Result<Self, String> {
        if !gain.is_finite() || !(0.0..=64.0).contains(&gain) {
            return Err("volume must be a finite linear gain within 0..=64".into());
        }
        if input_channels == 0
            || output_channels == 0
            || (input_channels != output_channels
                && (input_channels > 6 || !matches!(output_channels, 1 | 2)))
        {
            return Err("unsupported PCM channel conversion".into());
        }
        Ok(Self {
            output,
            gain,
            input_channels,
            output_channels,
            frame: [0.0; 6],
            filled: 0,
        })
    }
    pub fn frame_complete(&self) -> bool {
        self.filled == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_double_surround_frame_omits_lfe_and_applies_gain() {
        let source: Vec<_> = [1f64, 2., 3., 100., 4., 5.]
            .into_iter()
            .flat_map(f64::to_le_bytes)
            .collect();
        let mut bytes = Vec::new();
        let mut gain = PcmGain::new(&mut bytes, 0.5, 6, 2).unwrap();
        gain.write_all(&source[..24]).unwrap();
        assert!(!gain.frame_complete());
        gain.write_all(&source[24..]).unwrap();
        assert!(gain.frame_complete());
        let pcm: Vec<_> = bytes
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let k = std::f64::consts::FRAC_1_SQRT_2;
        assert_eq!(
            pcm,
            [(1. + k * 3. + k * 4.) * 0.5, (2. + k * 3. + k * 5.) * 0.5]
        );
    }
    #[test]
    fn invalid_parameters_are_rejected_before_writes() {
        let mut bytes = Vec::new();
        for (gain, input, output) in [
            (f64::NAN, 2, 2),
            (65., 2, 2),
            (1., 0, 1),
            (1., 7, 2),
            (1., 2, 3),
        ] {
            assert!(PcmGain::new(&mut bytes, gain, input, output).is_err());
        }
        assert!(bytes.is_empty());
        let mut gain = PcmGain::new(&mut bytes, 0., 8, 8).unwrap();
        gain.write_all(&[0u8; 64]).unwrap();
        assert!(gain.frame_complete());
    }
}
