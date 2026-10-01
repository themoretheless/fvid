//! Owned streaming windowed-sinc PCM resampling with an exact rational clock.
use std::{
    collections::VecDeque,
    io::{self, Write},
};

pub struct Resampler<W> {
    output: W,
    input_rate: u32,
    output_rate: u32,
    channels: usize,
    radius: i64,
    cutoff: f64,
    queue: VecDeque<[f32; 64]>,
    base: u64,
    count: u64,
    emitted: u64,
    first: [f32; 64],
    last: [f32; 64],
    frame: [f32; 64],
    filled: usize,
}
impl<W: Write> Resampler<W> {
    pub fn new(
        output: W,
        input_rate: u32,
        output_rate: u32,
        channels: u16,
    ) -> io::Result<Self> {
        if input_rate == 0 || output_rate == 0 || !(1..=64).contains(&channels) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid resampler geometry",
            ));
        }
        let ratio = (f64::from(output_rate) / f64::from(input_rate)).min(1.0);
        Ok(Self {
            output,
            input_rate,
            output_rate,
            channels: channels as usize,
            radius: (32.0 / ratio).ceil() as i64,
            cutoff: 0.94 * ratio,
            queue: VecDeque::new(),
            base: 0,
            count: 0,
            emitted: 0,
            first: [0.0; 64],
            last: [0.0; 64],
            frame: [0.0; 64],
            filled: 0,
        })
    }
    fn emit(&mut self, finalizing: bool) -> io::Result<()> {
        let total = (u128::from(self.count) * u128::from(self.output_rate))
            .div_ceil(u128::from(self.input_rate));
        while u128::from(self.emitted) < total {
            let position = u128::from(self.emitted) * u128::from(self.input_rate);
            let center = i64::try_from(position / u128::from(self.output_rate))
                .map_err(|_| io::Error::other("resampler clock overflow"))?;
            if !finalizing && center + self.radius >= self.count as i64 {
                break;
            }
            let fraction =
                (position % u128::from(self.output_rate)) as f64 / f64::from(self.output_rate);
            let mut sum = [0.0f64; 64];
            let mut normalization = 0.0;
            for offset in -self.radius..=self.radius {
                let distance = offset as f64 - fraction;
                if distance.abs() >= self.radius as f64 {
                    continue;
                }
                let angle = std::f64::consts::PI * distance / self.radius as f64;
                let window = 0.42 + 0.5 * angle.cos() + 0.08 * (2.0 * angle).cos();
                let phase = std::f64::consts::PI * self.cutoff * distance;
                let sinc = if phase.abs() < 1e-12 {
                    1.0
                } else {
                    phase.sin() / phase
                };
                let weight = self.cutoff * sinc * window;
                let index = center + offset;
                let sample = if index < 0 {
                    &self.first
                } else if index as u64 >= self.count {
                    &self.last
                } else {
                    self.queue
                        .get((index as u64 - self.base) as usize)
                        .ok_or_else(|| io::Error::other("resampler history unavailable"))?
                };
                normalization += weight;
                for channel in 0..self.channels {
                    sum[channel] += f64::from(sample[channel]) * weight;
                }
            }
            for value in &sum[..self.channels] {
                let value = (value / normalization) as f32;
                if !value.is_finite() {
                    return Err(io::Error::other("non-finite resampled PCM"));
                }
                self.output.write_all(&value.to_le_bytes())?;
            }
            self.emitted += 1;
            let next = (u128::from(self.emitted) * u128::from(self.input_rate)
                / u128::from(self.output_rate)) as u64;
            let keep = next.saturating_sub(self.radius as u64);
            while self.base < keep && !self.queue.is_empty() {
                self.queue.pop_front();
                self.base += 1;
            }
        }
        Ok(())
    }
    pub fn finish(&mut self) -> io::Result<u64> {
        if self.filled != 0 {
            return Err(io::Error::other("partial resampler channel frame"));
        }
        if self.input_rate != self.output_rate {
            self.emit(true)?;
        }
        self.output.flush()?;
        Ok(self.emitted)
    }
}
/// Drain produced PCM without disturbing the filter history or rational clock.
impl Resampler<Vec<u8>> {
    pub fn take_output(&mut self) -> Vec<u8> { std::mem::take(&mut self.output) }
}

impl<W: Write> Write for Resampler<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() % 4 != 0 {
            return Err(io::Error::other("partial resampler PCM sample"));
        }
        for sample in bytes.chunks_exact(4) {
            let sample = f32::from_le_bytes(sample.try_into().unwrap());
            if !sample.is_finite() {
                return Err(io::Error::other("non-finite resampler input"));
            }
            self.frame[self.filled] = sample;
            self.filled += 1;
            if self.filled == self.channels {
                if self.input_rate == self.output_rate {
                    for value in &self.frame[..self.channels] {
                        self.output.write_all(&value.to_le_bytes())?;
                    }
                    self.emitted += 1;
                } else {
                    if self.count == 0 {
                        self.first = self.frame;
                    }
                    self.last = self.frame;
                    self.queue.push_back(self.frame);
                }
                self.count += 1;
                self.filled = 0;
                if self.input_rate != self.output_rate {
                    self.emit(false)?;
                }
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn convert(samples: &[f32], input: u32, output: u32, channels: u16, chunk: usize) -> Vec<f32> {
        let data: Vec<_> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut bytes = Vec::new();
        let mut resampler = Resampler::new(&mut bytes, input, output, channels).unwrap();
        for part in data.chunks(chunk * 4) {
            resampler.write_all(part).unwrap();
        }
        let frames = resampler.finish().unwrap();
        assert_eq!(
            frames as u128,
            ((samples.len() / channels as usize) as u128 * output as u128).div_ceil(input as u128)
        );
        bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect()
    }
    #[test]
    fn exact_clock_dc_channel_isolation_and_chunk_independence() {
        let samples: Vec<_> = (0..997).flat_map(|_| [0.25, -0.75]).collect();
        for (input, output) in [(44100, 48000), (48000, 8000), (8000, 44100), (48000, 48000)] {
            let a = convert(&samples, input, output, 2, 1);
            assert_eq!(a, convert(&samples, input, output, 2, 137));
            for pair in a.chunks_exact(2) {
                assert!((pair[0] - 0.25).abs() < 1e-7);
                assert!((pair[1] + 0.75).abs() < 1e-7);
            }
        }
    }
    #[test]
    fn downsampling_rejects_out_of_band_tone_and_preserves_passband() {
        let tone = |hz: f64| {
            (0..4800)
                .map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / 48000.0).sin() as f32)
                .collect::<Vec<_>>()
        };
        let rms = |values: &[f32]| {
            (values[100..values.len() - 100]
                .iter()
                .map(|v| f64::from(*v).powi(2))
                .sum::<f64>()
                / (values.len() - 200) as f64)
                .sqrt()
        };
        let low = convert(&tone(1000.0), 48000, 16000, 1, 17);
        let high = convert(&tone(12000.0), 48000, 16000, 1, 17);
        assert!((rms(&low) - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.002);
        assert!(rms(&high) < 0.0001, "aliased RMS {}", rms(&high));
    }
}
