//! Streaming K-weighting for loudness measurement, independent of media backends.
#[derive(Clone, Default)]
struct State {
    x: [f64; 2],
    y: [f64; 2],
}
#[derive(Clone, Copy)]
struct Coefficients {
    b: [f64; 3],
    a: [f64; 2],
}
impl Coefficients {
    fn sample(self, s: &mut State, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * s.x[0] + self.b[2] * s.x[1]
            - self.a[0] * s.y[0]
            - self.a[1] * s.y[1];
        s.x = [x, s.x[0]];
        s.y = [y, s.y[0]];
        y
    }
}
/// Two cascaded biquads with independent state for every interleaved channel.
/// This is the weighting stage, not an integrated LUFS/true-peak meter.
pub struct KWeighting {
    shelf: Coefficients,
    high_pass: Coefficients,
    states: Vec<(State, State)>,
}
impl KWeighting {
    pub fn new(sample_rate: u32, channels: usize) -> Result<Self, String> {
        if !(8000..=384000).contains(&sample_rate) || !(1..=64).contains(&channels) {
            return Err("K-weighting requires 8–384 kHz and 1–64 channels".into());
        }
        let k = (std::f64::consts::PI * 1681.974450955533 / f64::from(sample_rate)).tan();
        let q = 0.7071752369554196;
        let h = 10f64.powf(3.999843853973347 / 20.0);
        let m = h.powf(0.4996667741545416);
        let d = 1.0 + k / q + k * k;
        let shelf = Coefficients {
            b: [
                (h + m * k / q + k * k) / d,
                2.0 * (k * k - h) / d,
                (h - m * k / q + k * k) / d,
            ],
            a: [2.0 * (k * k - 1.0) / d, (1.0 - k / q + k * k) / d],
        };
        let k = (std::f64::consts::PI * 38.13547087602444 / f64::from(sample_rate)).tan();
        let q = 0.5003270373238773;
        let d = 1.0 + k / q + k * k;
        let high_pass = Coefficients {
            b: [1.0, -2.0, 1.0],
            a: [2.0 * (k * k - 1.0) / d, (1.0 - k / q + k * k) / d],
        };
        Ok(Self {
            shelf,
            high_pass,
            states: vec![(State::default(), State::default()); channels],
        })
    }
    /// Reject malformed input before mutating samples or filter state.
    pub fn process(&mut self, pcm: &mut [f64]) -> Result<(), String> {
        if pcm.len() % self.states.len() != 0
            || pcm.iter().any(|x| !x.is_finite() || x.abs() > 1e100)
        {
            return Err(
                "K-weighting requires complete finite PCM frames within numeric range".into(),
            );
        }
        for frame in pcm.chunks_exact_mut(self.states.len()) {
            for (sample, (shelf, high_pass)) in frame.iter_mut().zip(&mut self.states) {
                *sample = self
                    .high_pass
                    .sample(high_pass, self.shelf.sample(shelf, *sample));
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_boundaries_channels_and_rejected_input_preserve_state() {
        for rate in [8000, 44100, 48000, 96000, 192000] {
            let input: Vec<f64> = (0..rate)
                .flat_map(|i| [if i == 0 { 1.0 } else { 0.0 }, 0.0])
                .collect();
            let mut whole = input.clone();
            KWeighting::new(rate, 2)
                .unwrap()
                .process(&mut whole)
                .unwrap();
            let mut split = input;
            let mut filter = KWeighting::new(rate, 2).unwrap();
            assert!(filter.process(&mut [f64::NAN, 0.0]).is_err());
            assert!(filter.process(&mut [1.0]).is_err());
            for part in split.chunks_mut(34) {
                filter.process(part).unwrap();
            }
            assert_eq!(whole, split);
            assert!(whole
                .chunks_exact(2)
                .all(|p| p[1] == 0.0 && p[0].is_finite()));
            assert!(whole[whole.len() - 2].abs() < 1e-8);
        }
    }
}

#[cfg(test)]
mod reference_tests {
    use super::*;
    #[test]
    #[ignore = "requires FVID_REFERENCE_FFMPEG"]
    fn agrees_with_independent_48khz_biquad_filter() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let pcm: Vec<f64> = (0..8192)
            .map(|i| {
                if i == 0 {
                    1.0
                } else {
                    (i as f64 * 0.13).sin() * 0.2
                }
            })
            .collect();
        let mut expected = Command::new(std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap())
            .args(["-v", "error", "-f", "f64le", "-ar", "48000", "-ac", "1", "-i", "pipe:0", "-af",
                "biquad=b0=1.53512485958697:b1=-2.69169618940638:b2=1.19839281085285:a0=1:a1=-1.69065929318241:a2=0.73248077421585:precision=f64,biquad=b0=1:b1=-2:b2=1:a0=1:a1=-1.99004745483398:a2=0.99007225036621:precision=f64",
                "-f", "f64le", "pipe:1"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let bytes: Vec<u8> = pcm.iter().flat_map(|x| x.to_le_bytes()).collect();
        let mut input = expected.stdin.take().unwrap();
        let writer = std::thread::spawn(move || input.write_all(&bytes).unwrap());
        let result = expected.wait_with_output().unwrap();
        writer.join().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut actual = pcm;
        KWeighting::new(48000, 1)
            .unwrap()
            .process(&mut actual)
            .unwrap();
        assert_eq!(result.stdout.len(), actual.len() * 8);
        for (i, (actual, expected)) in actual.iter().zip(result.stdout.chunks_exact(8)).enumerate()
        {
            let expected = f64::from_le_bytes(expected.try_into().unwrap());
            assert!(
                (actual - expected).abs() < 1e-8,
                "sample {i}: {actual} != {expected}"
            );
        }
    }
}
