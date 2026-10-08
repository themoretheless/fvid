//! Owned complex SBR HF assembly with smoothing/noise/sine frame state.
use super::{
    Result, aac_sbr_gain::Levels, aac_sbr_grid::TimeGrid, aac_sbr_noise_table::NOISE,
    aac_sbr_qmf::Complex, invalid,
};
const SMOOTH: [f64; 5] = [
    0.33333333333333,
    0.30150283239582,
    0.21816949906249,
    0.11516383427084,
    0.03183050093751,
];
#[derive(Clone, Debug, PartialEq)]
pub struct Assembly {
    gain: [[f64; 64]; 4],
    noise: [[f64; 64]; 4],
    geometry: Option<(u8, usize)>,
    noise_index: usize,
    sine_index: usize,
}
impl Default for Assembly {
    fn default() -> Self {
        Self {
            gain: [[0.0; 64]; 4],
            noise: [[0.0; 64]; 4],
            geometry: None,
            noise_index: 0,
            sine_index: 0,
        }
    }
}
impl Assembly {
    /// Clear all history/phases for a fresh decoder or seek checkpoint reset.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Input/output rows use tHFAdj as origin. Output contains only the adjusted
    /// high range; the frame engine subsequently merges the delayed low range.
    /// `disable_smoothing` is bs_smoothing_mode. `header_reset` primes gain/noise
    /// history and restarts the noise index, but preserves sinusoidal phase.
    /// Failure, including late arithmetic overflow, leaves the whole state intact.
    pub fn process(
        &mut self,
        high: &[[Complex; 64]],
        slots: u8,
        grid: &TimeGrid,
        kx: u8,
        levels: &[Vec<Levels>],
        suppress_noise: &[bool],
        disable_smoothing: bool,
        header_reset: bool,
    ) -> Result<Vec<[Complex; 64]>> {
        if !matches!(slots, 15 | 16)
            || high.len() != 2 * usize::from(slots) + 6
            || grid.envelope.len() < 2
            || grid.envelope.len() > 6
            || grid.envelope.windows(2).any(|b| b[0] >= b[1])
            || usize::from(*grid.envelope.last().unwrap()) > usize::from(slots) + 3
            || levels.len() + 1 != grid.envelope.len()
            || suppress_noise.len() != levels.len()
            || levels[0].is_empty()
            || levels[0].len() > 64
            || kx == 0
            || usize::from(kx) + levels[0].len() > 64
            || levels.iter().any(|r| r.len() != levels[0].len())
            || levels.iter().flatten().any(|l| {
                [l.gain, l.noise, l.sine]
                    .iter()
                    .any(|x| !x.is_finite() || *x < 0.0)
            })
            || high
                .iter()
                .flatten()
                .any(|x| !x.re.is_finite() || !x.im.is_finite())
        {
            return Err(invalid("invalid SBR assembly inputs"));
        }
        let width = levels[0].len();
        let start_band = usize::from(kx);
        let mut trial = self.clone();
        if header_reset {
            let phase = trial.sine_index;
            trial = Self::default();
            trial.sine_index = phase;
        }
        if let Some(old) = trial.geometry {
            if old != (kx, width) {
                return Err(invalid(
                    "SBR assembly geometry changed without header reset",
                ));
            }
        } else {
            trial.geometry = Some((kx, width));
            for (m, level) in levels[0].iter().enumerate() {
                for h in 0..4 {
                    trial.gain[h][start_band + m] = level.gain;
                    trial.noise[h][start_band + m] = level.noise;
                }
            }
        }
        let mut output = vec![[Complex::default(); 64]; high.len()];
        for (envelope, row) in levels.iter().enumerate() {
            for t in 2 * usize::from(grid.envelope[envelope])
                ..2 * usize::from(grid.envelope[envelope + 1])
            {
                for (m, level) in row.iter().enumerate() {
                    let k = start_band + m;
                    let blend = |current: f64, history: &[[f64; 64]; 4]| {
                        if disable_smoothing {
                            current
                        } else {
                            (1..=4).fold(SMOOTH[0] * current, |v, j| {
                                v + SMOOTH[j] * history[j - 1][k]
                            })
                        }
                    };
                    let gain = if suppress_noise[envelope] {
                        level.gain
                    } else {
                        blend(level.gain, &trial.gain)
                    };
                    let noise = if suppress_noise[envelope] || level.sine != 0.0 {
                        0.0
                    } else {
                        blend(level.noise, &trial.noise)
                    };
                    let random = NOISE[(trial.noise_index + m + 1) % 512];
                    let sine_re = [1.0, 0.0, -1.0, 0.0][trial.sine_index];
                    let sine_im = [0.0, 1.0, 0.0, -1.0][trial.sine_index]
                        * if k % 2 == 0 { 1.0 } else { -1.0 };
                    let value = Complex {
                        re: gain * high[t][k].re + noise * random.re + level.sine * sine_re,
                        im: gain * high[t][k].im + noise * random.im + level.sine * sine_im,
                    };
                    if !value.re.is_finite() || !value.im.is_finite() {
                        return Err(invalid("SBR assembly arithmetic overflow"));
                    }
                    output[t][k] = value;
                }
                for h in (1..4).rev() {
                    trial.gain[h] = trial.gain[h - 1];
                    trial.noise[h] = trial.noise[h - 1];
                }
                for (m, level) in row.iter().enumerate() {
                    trial.gain[0][start_band + m] = level.gain;
                    trial.noise[0][start_band + m] = level.noise;
                }
                trial.noise_index = (trial.noise_index + width) % 512;
                trial.sine_index = (trial.sine_index + 1) % 4;
            }
        }
        *self = trial;
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_checked_protocol_noise_constants_match_saved_values() {
        let bytes = include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-noise-protocol.f64le"
        );
        assert_eq!(bytes.len(), 512 * 16);
        for (i, value) in NOISE.iter().enumerate() {
            let re = f64::from_le_bytes(bytes[i * 16..i * 16 + 8].try_into().unwrap());
            let im = f64::from_le_bytes(bytes[i * 16 + 8..i * 16 + 16].try_into().unwrap());
            assert_eq!(value.re.to_bits(), re.to_bits());
            assert_eq!(value.im.to_bits(), im.to_bits());
        }
    }
    #[test]
    fn independent_decimal_multiframe_assembly_covers_smoothing_attack_reset_and_noise_wrap() {
        let metadata: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-assembly-oracles.json"
        ))
        .unwrap();
        let bytes = include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-assembly-decimal.f64le"
        );
        let mut offset = 0;
        let read = |offset: &mut usize| {
            let v = f64::from_le_bytes(bytes[*offset..*offset + 8].try_into().unwrap());
            *offset += 8;
            v
        };
        let cases = metadata["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 4);
        for case in cases {
            let slots = case["slots"].as_u64().unwrap() as u8;
            let disabled = case["disabled"].as_bool().unwrap();
            let mut state = Assembly::default();
            for frame in case["frames"].as_array().unwrap() {
                let grid = TimeGrid {
                    envelope: frame["envelope"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_u64().unwrap() as u8)
                        .collect(),
                    noise: vec![],
                };
                let levels: Vec<Vec<Levels>> = frame["levels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| {
                        r.as_array()
                            .unwrap()
                            .iter()
                            .map(|l| Levels {
                                gain: l[0].as_f64().unwrap(),
                                noise: l[1].as_f64().unwrap(),
                                sine: l[2].as_f64().unwrap(),
                            })
                            .collect()
                    })
                    .collect();
                let suppress: Vec<_> = frame["suppress"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_bool().unwrap())
                    .collect();
                let reset = frame["reset"].as_bool().unwrap();
                let mut high = vec![[Complex::default(); 64]; 2 * usize::from(slots) + 6];
                for r in &mut high {
                    for x in &mut r[11..16] {
                        *x = Complex {
                            re: read(&mut offset),
                            im: read(&mut offset),
                        };
                    }
                }
                let mut checkpoint = state.clone();
                let output = state
                    .process(&high, slots, &grid, 11, &levels, &suppress, disabled, reset)
                    .unwrap();
                assert_eq!(
                    output,
                    checkpoint
                        .process(&high, slots, &grid, 11, &levels, &suppress, disabled, reset)
                        .unwrap()
                );
                for r in output {
                    assert!(
                        r[..11]
                            .iter()
                            .chain(&r[16..])
                            .all(|x| *x == Complex::default())
                    );
                    for x in &r[11..16] {
                        let re = read(&mut offset);
                        let im = read(&mut offset);
                        assert!((x.re - re).abs() < 2e-12);
                        assert!((x.im - im).abs() < 2e-12);
                    }
                }
                assert_eq!(
                    state.noise_index,
                    frame["noise_end"].as_u64().unwrap() as usize
                );
                assert_eq!(
                    state.sine_index,
                    frame["sine_end"].as_u64().unwrap() as usize
                );
            }
        }
        assert_eq!(offset, bytes.len());
    }
    #[test]
    fn phase_parity_seek_reset_and_failures_are_transactional() {
        let high = vec![[Complex::default(); 64]; 38];
        let grid = TimeGrid {
            envelope: vec![0, 16],
            noise: vec![],
        };
        let levels = vec![vec![Levels {
            gain: 0.0,
            noise: 0.0,
            sine: 1.0,
        }]];
        let mut state = Assembly::default();
        let result = state
            .process(&high, 16, &grid, 11, &levels, &[false], true, false)
            .unwrap();
        assert_eq!(
            &result[..4].iter().map(|r| r[11]).collect::<Vec<_>>(),
            &vec![
                Complex { re: 1.0, im: 0.0 },
                Complex { re: 0.0, im: -1.0 },
                Complex { re: -1.0, im: 0.0 },
                Complex { re: 0.0, im: 1.0 }
            ]
        );
        let checkpoint = state.clone();
        let mut bad = high.clone();
        bad[31][11].re = f64::NAN;
        assert!(
            state
                .process(&bad, 16, &grid, 11, &levels, &[false], false, true)
                .is_err()
        );
        assert_eq!(state, checkpoint);
        let mut bad = high.clone();
        bad[31][11].re = f64::MAX;
        let amplified = vec![vec![Levels {
            gain: 2.0,
            noise: 0.0,
            sine: 0.0,
        }]];
        assert!(
            state
                .process(&bad, 16, &grid, 11, &amplified, &[false], true, false)
                .is_err()
        );
        assert_eq!(state, checkpoint);
        assert!(
            state
                .process(&high, 16, &grid, 12, &levels, &[false], false, false)
                .is_err()
        );
        assert_eq!(state, checkpoint);
        state.reset();
        assert_eq!(state, Assembly::default());
        let noise = vec![vec![Levels {
            gain: 0.0,
            noise: 1.0,
            sine: 0.0,
        }]];
        let output = state
            .process(&high, 16, &grid, 11, &noise, &[false], true, false)
            .unwrap();
        assert_eq!(output[0][11], NOISE[1]);
        for envelope in [vec![], vec![0], vec![0, 0], vec![0, 20]] {
            assert!(
                state
                    .process(
                        &high,
                        16,
                        &TimeGrid {
                            envelope,
                            noise: vec![]
                        },
                        11,
                        &noise,
                        &[false],
                        true,
                        false
                    )
                    .is_err()
            );
        }
    }
}
