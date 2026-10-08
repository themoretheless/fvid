//! Owned SBR energy/noise dequantization and coupled stereo reconstruction.
use super::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stereo {
    pub left: f64,
    pub right: f64,
}
fn power(exponent: f64) -> Result<f64> {
    let value = exponent.exp2();
    if !value.is_finite() || value <= 0.0 {
        return Err(invalid("unrepresentable SBR dequantized value"));
    }
    Ok(value)
}
/// Quantized envelope energy: 64 * 2^(E/a), a=2 at 1.5 dB, a=1 at 3 dB.
pub fn envelope(level: i16, coarse: bool) -> Result<f64> {
    power(6.0 + f64::from(level) / if coarse { 1.0 } else { 2.0 })
}
/// Quantized noise-to-signal ratio: 2^(6-Q), with normative Q in 0..=30.
pub fn noise(level: i16) -> Result<f64> {
    if !(0..=30).contains(&level) {
        return Err(invalid("SBR noise level exceeds normative range"));
    }
    power(6.0 - f64::from(level))
}
fn split(total: f64, score: f64) -> Result<Stereo> {
    // Calculate both independently: subtracting the dominant channel from total
    // would lose the quiet channel at strongly asymmetric panning.
    let left = total / (1.0 + power(-score)?);
    let right = total / (1.0 + power(score)?);
    if !left.is_finite() || !right.is_finite() || left <= 0.0 || right <= 0.0 {
        return Err(invalid("unrepresentable SBR stereo channel value"));
    }
    Ok(Stereo { left, right })
}
/// Balance inputs are reconstructed values (already multiplied by two).
pub fn coupled_envelope(level: i16, balance: i16, coarse: bool) -> Result<Stereo> {
    if balance % 2 != 0 {
        return Err(invalid("SBR envelope balance must be even"));
    }
    let divisor = if coarse { 1.0 } else { 2.0 };
    let pan = if coarse { 12.0 } else { 24.0 };
    let total = power(7.0 + f64::from(level) / divisor)?;
    split(total, (f64::from(balance) - pan) / divisor)
}
pub fn coupled_noise(level: i16, balance: i16) -> Result<Stereo> {
    if !(0..=24).contains(&balance) || balance % 2 != 0 {
        return Err(invalid(
            "SBR noise balance exceeds normative range or parity",
        ));
    }
    // Noise has the opposite balance orientation to envelope energy.
    split(2.0 * noise(level)?, 12.0 - f64::from(balance))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= expected.abs() * 3e-14,
            "{actual} != {expected}"
        );
    }
    #[test]
    fn all_values_match_independent_high_precision_decimal_oracles() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-dequant-oracles.json"
        ))
        .unwrap();
        for c in cases.as_array().unwrap() {
            let level = c["level"].as_i64().unwrap() as i16;
            let coarse = c["coarse"].as_bool().unwrap_or(false);
            let expected = |name: &str| c[name].as_str().unwrap().parse::<f64>().unwrap();
            match c["kind"].as_str().unwrap() {
                "envelope" => close(envelope(level, coarse).unwrap(), expected("value")),
                "noise" => close(noise(level).unwrap(), expected("value")),
                kind => {
                    let balance = c["balance"].as_i64().unwrap() as i16;
                    let value = if kind == "coupled_envelope" {
                        coupled_envelope(level, balance, coarse).unwrap()
                    } else {
                        coupled_noise(level, balance).unwrap()
                    };
                    close(value.left, expected("left"));
                    close(value.right, expected("right"));
                }
            }
        }
    }
    #[test]
    fn centered_stereo_symmetry_energy_and_noise_orientation_are_explicit() {
        for coarse in [false, true] {
            let center = if coarse { 12 } else { 24 };
            for level in [-8, 0, 1, 30, 127] {
                let value = coupled_envelope(level, center, coarse).unwrap();
                close(value.left, envelope(level, coarse).unwrap());
                close(value.right, value.left);
                for balance in [0, 2, 12, 24, 48] {
                    let a = coupled_envelope(level, balance, coarse).unwrap();
                    let b = coupled_envelope(level, 2 * center - balance, coarse).unwrap();
                    close(a.left, b.right);
                    close(a.right, b.left);
                    close(a.left + a.right, 2.0 * envelope(level, coarse).unwrap());
                }
            }
        }
        let e = coupled_envelope(0, 14, true).unwrap();
        close(e.left, 102.4);
        close(e.right, 25.6);
        let n = coupled_noise(6, 14).unwrap();
        close(n.left, 0.4);
        close(n.right, 1.6);
    }
    #[test]
    fn normative_noise_bounds_even_balance_and_numeric_extremes_are_checked() {
        for level in -2..=32 {
            assert_eq!(noise(level).is_ok(), (0..=30).contains(&level));
            for balance in -2..=26 {
                assert_eq!(
                    coupled_noise(level, balance).is_ok(),
                    (0..=30).contains(&level) && (0..=24).contains(&balance) && balance % 2 == 0
                );
            }
        }
        assert!(coupled_envelope(0, 13, true).is_err());
        for level in [i16::MIN, i16::MAX] {
            assert!(envelope(level, false).is_err());
            assert!(coupled_envelope(level, 12, true).is_err());
        }
    }
}
