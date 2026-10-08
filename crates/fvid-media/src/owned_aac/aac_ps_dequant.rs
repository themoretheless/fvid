//! PS quantization grids, GOST R 53556.8-2013 tables 25/26/28/31.
//! Physical IID is in dB, ICC is coherence, IPD/OPD are radians. These are
//! parameters for the actual complex mixing stage, not a stereo PCM substitute.
use super::{Result, aac_ps_data::IidMode, invalid};

const IID_COARSE: [i16; 15] = [-25, -18, -14, -10, -7, -4, -2, 0, 2, 4, 7, 10, 14, 18, 25];
const IID_FINE: [i16; 31] = [
    -50, -45, -40, -35, -30, -25, -22, -19, -16, -13, -10, -8, -6, -4, -2, 0, 2, 4, 6, 8, 10, 13,
    16, 19, 22, 25, 30, 35, 40, 45, 50,
];
const ICC: [f64; 8] = [1.0, 0.937, 0.84118, 0.60092, 0.36764, 0.0, -0.589, -1.0];

pub fn iid_db(mode: IidMode, index: i16) -> Result<f64> {
    let (table, offset): (&[i16], i16) = if mode.fine() {
        (&IID_FINE, 15)
    } else {
        (&IID_COARSE, 7)
    };
    if !(-offset..=offset).contains(&index) {
        return Err(invalid("PS IID index exceeds quantization grid"));
    }
    Ok(f64::from(table[(index + offset) as usize]))
}

pub fn coherence(index: i16) -> Result<f64> {
    if !(0..=7).contains(&index) {
        return Err(invalid("PS ICC index exceeds quantization grid"));
    }
    Ok(ICC[index as usize])
}

pub fn phase_radians(index: i16) -> Result<f64> {
    if !(0..=7).contains(&index) {
        return Err(invalid("PS phase index exceeds quantization grid"));
    }
    Ok(f64::from(index) * std::f64::consts::FRAC_PI_4)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_quantization_level_matches_saved_normative_and_decimal_phase_oracles() {
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ps-dequant-oracles.json"
        ))
        .unwrap();
        for mode in 0..6 {
            let mode = IidMode::new(mode).unwrap();
            let key = if mode.fine() {
                "iid_fine"
            } else {
                "iid_coarse"
            };
            for row in oracle[key].as_array().unwrap() {
                let index = row["index"].as_i64().unwrap() as i16;
                let expected = row["db"].as_str().unwrap().parse::<f64>().unwrap();
                assert_eq!(iid_db(mode, index).unwrap(), expected);
                assert_eq!(iid_db(mode, -index).unwrap(), -expected);
            }
        }
        for row in oracle["icc"].as_array().unwrap() {
            let index = row["index"].as_i64().unwrap() as i16;
            assert_eq!(
                coherence(index).unwrap(),
                row["value"].as_str().unwrap().parse::<f64>().unwrap()
            );
        }
        for row in oracle["phase"].as_array().unwrap() {
            let index = row["index"].as_i64().unwrap() as i16;
            let expected = row["value"].as_str().unwrap().parse::<f64>().unwrap();
            assert!((phase_radians(index).unwrap() - expected).abs() <= 2e-15);
        }
        for index in [i16::MIN, -16, -8, -1, 8, 16, i16::MAX] {
            for mode in 0..6 {
                let mode = IidMode::new(mode).unwrap();
                assert_eq!(
                    iid_db(mode, index).is_ok(),
                    if mode.fine() {
                        (-15..=15).contains(&index)
                    } else {
                        (-7..=7).contains(&index)
                    }
                );
            }
            assert_eq!(coherence(index).is_ok(), (0..=7).contains(&index));
            assert_eq!(phase_radians(index).is_ok(), (0..=7).contains(&index));
        }
    }
}
