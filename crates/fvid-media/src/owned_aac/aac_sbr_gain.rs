//! Owned SBR signal levels and limiter/boost amplitudes, with Cor.1 fixes.
use super::{Result, invalid};
#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub target: f64,
    pub current: f64,
    pub noise_ratio: f64,
    /// S_Mapped: harmonic presence anywhere in the envelope frequency band.
    pub harmonic_band: bool,
    /// S_IndexMapped: only the actual QMF channel carrying the sinusoid.
    pub harmonic_line: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Levels {
    pub gain: f64,
    pub noise: f64,
    pub sine: f64,
}
fn valid(bands: &[Band]) -> bool {
    !bands.is_empty()
        && bands.len() <= 64
        && bands.iter().all(|b| {
            [b.target, b.current, b.noise_ratio]
                .iter()
                .all(|x| x.is_finite() && *x >= 0.0)
                && (!b.harmonic_line || b.harmonic_band)
        })
}
/// Pre-limiter amplitudes in the standard's QMF/energy units (epsilon=1,
/// Cor.1). `suppress_noise` is true at l_A or the carried l_APrev envelope.
/// Harmonic presence affects gain over a band, but sine amplitude is nonzero
/// only at a marked QMF line, as corrected by ISO Cor.1 page 11.
pub fn calculate(bands: &[Band], suppress_noise: bool) -> Result<Vec<Levels>> {
    if !valid(bands) {
        return Err(invalid("invalid SBR gain inputs"));
    }
    Ok(bands
        .iter()
        .map(|b| {
            let inverse = 1.0 / (1.0 + b.noise_ratio);
            let noise_fraction = b.noise_ratio / (1.0 + b.noise_ratio);
            let gain_fraction = if b.harmonic_band {
                noise_fraction
            } else if suppress_noise {
                1.0
            } else {
                inverse
            };
            Levels {
                gain: b.target.sqrt() * gain_fraction.sqrt() / (1.0 + b.current).sqrt(),
                noise: (b.target * noise_fraction).sqrt(),
                sine: if b.harmonic_line {
                    (b.target * inverse).sqrt()
                } else {
                    0.0
                },
            }
        })
        .collect())
}
/// Apply amplitude limiting, proportional noise reduction and bandwise boost.
/// Absolute limiter borders must partition all supplied QMF bands.
pub fn limit(
    bands: &[Band],
    levels: &[Levels],
    kx: u8,
    borders: &[u8],
    mode: u8,
    suppress_noise: bool,
) -> Result<Vec<Levels>> {
    if !valid(bands)
        || levels.len() != bands.len()
        || mode > 3
        || kx == 0
        || usize::from(kx) + bands.len() > 64
        || borders.len() < 2
        || borders.len() > 65
        || borders[0] != kx
        || usize::from(*borders.last().unwrap()) != usize::from(kx) + bands.len()
        || borders.windows(2).any(|x| x[0] >= x[1])
        || levels.iter().any(|l| {
            [l.gain, l.noise, l.sine]
                .iter()
                .any(|x| !x.is_finite() || *x < 0.0)
        })
        || levels.iter().zip(bands).any(|(l, b)| {
            [l.gain, l.noise, l.sine]
                .iter()
                .any(|x| *x > b.target.sqrt())
        })
    {
        return Err(invalid("invalid SBR limiter level inputs"));
    }
    let factor = [0.70795, 1.0, 1.41254, 1e10][usize::from(mode)];
    let mut output = levels.to_vec();
    for border in borders.windows(2) {
        let start = usize::from(border[0] - kx);
        let end = usize::from(border[1] - kx);
        let scale = bands[start..end]
            .iter()
            .fold(1e-12f64, |s, b| s.max(b.target).max(b.current));
        let epsilon = 1e-12 / scale;
        let original = bands[start..end]
            .iter()
            .map(|b| b.target / scale)
            .sum::<f64>()
            + epsilon;
        let current = bands[start..end]
            .iter()
            .map(|b| b.current / scale)
            .sum::<f64>()
            + epsilon;
        let maximum = (original.sqrt() / current.sqrt() * factor).min(1e5);
        for value in &mut output[start..end] {
            if value.gain > maximum {
                value.noise *= maximum / value.gain;
                value.gain = maximum;
            }
        }
        // Work in common amplitude units; direct current*gain^2 and sine^2
        // could overflow despite a representable final boost ratio.
        let amplitude = bands[start..end]
            .iter()
            .map(|b| b.target.sqrt().max(b.current.sqrt()))
            .chain(output[start..end].iter().map(|l| l.noise.max(l.sine)))
            .fold(1e-6f64, f64::max);
        let epsilon = (1e-6 / amplitude).powi(2);
        let original = bands[start..end]
            .iter()
            .map(|b| (b.target.sqrt() / amplitude).powi(2))
            .sum::<f64>()
            + epsilon;
        let mut reconstructed = epsilon;
        for (band, value) in bands[start..end].iter().zip(&output[start..end]) {
            reconstructed += (band.current.sqrt() / amplitude * value.gain).powi(2)
                + (value.sine / amplitude).powi(2);
            if !suppress_noise && value.sine == 0.0 {
                reconstructed += (value.noise / amplitude).powi(2);
            }
        }
        let boost = (original.sqrt() / reconstructed.sqrt()).min(1.584893192);
        for value in &mut output[start..end] {
            value.gain *= boost;
            value.noise *= boost;
            value.sine *= boost;
        }
    }
    if output
        .iter()
        .any(|l| [l.gain, l.noise, l.sine].iter().any(|x| !x.is_finite()))
    {
        return Err(invalid("SBR boosted amplitude overflow"));
    }
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_decimal_levels_and_all_limiter_modes_match() {
        let data: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-gain-decimal.json"
        ))
        .unwrap();
        let rows = data["cases"].as_array().unwrap();
        assert_eq!(rows.len(), 60);
        for row in rows {
            let bands: Vec<_> = row["bands"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| Band {
                    target: x[0].as_f64().unwrap(),
                    current: x[1].as_f64().unwrap(),
                    noise_ratio: x[2].as_f64().unwrap(),
                    harmonic_band: x[3].as_bool().unwrap(),
                    harmonic_line: x[4].as_bool().unwrap(),
                })
                .collect();
            let suppress = row["suppress"].as_bool().unwrap();
            let mut actual = calculate(&bands, suppress).unwrap();
            if let Some(mode) = row["mode"].as_u64() {
                let borders: Vec<_> = row["borders"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_u64().unwrap() as u8)
                    .collect();
                actual = limit(&bands, &actual, 10, &borders, mode as u8, suppress).unwrap();
            }
            for (value, reference) in actual.iter().zip(row["expected"].as_array().unwrap()) {
                for (a, b) in [value.gain, value.noise, value.sine]
                    .into_iter()
                    .zip(reference.as_array().unwrap())
                {
                    let b = b.as_f64().unwrap();
                    assert!(
                        (a - b).abs() < 2e-12 * b.abs().max(1e-20),
                        "{a} != {b}; case {row}"
                    );
                }
            }
        }
    }
    #[test]
    fn harmonic_line_is_distinct_from_presence_and_attack_suppresses_only_noise_gain_term() {
        let base = Band {
            target: 64.0,
            current: 15.0,
            noise_ratio: 1.0,
            harmonic_band: false,
            harmonic_line: false,
        };
        let plain = calculate(&[base], false).unwrap()[0];
        let attack = calculate(&[base], true).unwrap()[0];
        assert!((plain.gain - 2f64.sqrt()).abs() < 1e-15);
        assert_eq!(attack.gain, 2.0);
        assert_eq!(plain.noise, attack.noise);
        let marked = Band {
            harmonic_band: true,
            ..base
        };
        let line = Band {
            harmonic_line: true,
            ..marked
        };
        let values = calculate(&[marked, line], false).unwrap();
        assert_eq!(values[0].gain, values[1].gain);
        assert_eq!(values[0].sine, 0.0);
        assert!(values[1].sine > 0.0);
        let zeros = Band {
            target: 0.0,
            current: 0.0,
            noise_ratio: 0.0,
            ..base
        };
        for mode in 0..=3 {
            assert_eq!(
                limit(
                    &[zeros],
                    &calculate(&[zeros], false).unwrap(),
                    10,
                    &[10, 11],
                    mode,
                    false
                )
                .unwrap(),
                vec![Levels::default()]
            );
        }
        let huge = Band {
            target: f64::MAX,
            current: f64::MAX,
            noise_ratio: f64::MAX,
            ..base
        };
        let raw = calculate(&[huge; 64], false).unwrap();
        let result = limit(&[huge; 64], &raw, 0, &[0, 64], 3, false);
        assert!(result.is_err());
        let raw = calculate(&[huge; 54], false).unwrap();
        assert!(
            limit(&[huge; 54], &raw, 10, &[10, 64], 3, false)
                .unwrap()
                .iter()
                .all(|x| x.gain.is_finite() && x.noise.is_finite())
        );
    }
    #[test]
    fn invalid_public_inputs_are_rejected() {
        let base = Band {
            target: 64.0,
            current: 15.0,
            noise_ratio: 1.0,
            harmonic_band: false,
            harmonic_line: false,
        };
        assert!(calculate(&[], false).is_err());
        assert!(calculate(&[base; 65], false).is_err());
        for bad in [-1.0, f64::NAN, f64::INFINITY] {
            for band in [
                Band {
                    target: bad,
                    ..base
                },
                Band {
                    current: bad,
                    ..base
                },
                Band {
                    noise_ratio: bad,
                    ..base
                },
            ] {
                assert!(calculate(&[band], false).is_err());
            }
        }
        assert!(
            calculate(
                &[Band {
                    harmonic_line: true,
                    ..base
                }],
                false
            )
            .is_err()
        );
        let levels = calculate(&[base], false).unwrap();
        assert!(
            limit(
                &[base],
                &[Levels {
                    gain: f64::MAX,
                    ..levels[0]
                }],
                10,
                &[10, 11],
                0,
                false
            )
            .is_err()
        );
        for borders in [vec![], vec![10], vec![10, 10], vec![9, 11], vec![10, 12]] {
            assert!(limit(&[base], &levels, 10, &borders, 0, false).is_err());
        }
        assert!(limit(&[base], &levels, 10, &[10, 11], 4, false).is_err());
        assert!(limit(&[base], &[], 10, &[10, 11], 0, false).is_err());
        assert!(
            limit(
                &[base],
                &[Levels {
                    noise: f64::NAN,
                    ..levels[0]
                }],
                10,
                &[10, 11],
                0,
                false
            )
            .is_err()
        );
    }
}
