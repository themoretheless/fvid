//! Owned real PS stereo matrices, GOST R 53556.8-2013 6.4.6.2.
//! Real matrices and phase primitives. Stateful temporal interpolation and
//! hybrid/decorrelation/synthesis are separate stages.
use super::{
    Result,
    aac_ps_data::{IccMode, IidMode},
    aac_ps_dequant,
    aac_ps_mapping::Indices,
    aac_sbr_qmf::Complex,
    invalid,
};
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Matrix {
    pub h11: f64,
    pub h12: f64,
    pub h21: f64,
    pub h22: f64,
}
impl Matrix {
    pub fn coefficients(self) -> [f64; 4] {
        [self.h11, self.h12, self.h21, self.h22]
    }
}
/// Quantized inputs keep physical domains bounded and exclude NaN/Infinity.
pub fn matrix(iid_mode: IidMode, iid: i16, icc_mode: IccMode, icc: i16) -> Result<Matrix> {
    let db = aac_ps_dequant::iid_db(iid_mode, iid)?;
    let rho = aac_ps_dequant::coherence(icc)?;
    let c = 10_f64.powf(db / 20.0);
    let root = std::f64::consts::SQRT_2;
    let [h11, h12, h21, h22] = if !icc_mode.mixing_b() {
        let c1 = root / (1.0 + c * c).sqrt();
        let c2 = c * c1;
        let alpha = rho.acos() / 2.0;
        let beta = alpha * (c1 - c2) / root;
        let (sa, ca) = (alpha + beta).sin_cos();
        let (sb, cb) = (beta - alpha).sin_cos();
        [ca * c2, cb * c1, sa * c2, sb * c1]
    } else {
        let rho = rho.max(0.05);
        let alpha = if c == 1.0 {
            std::f64::consts::FRAC_PI_4
        } else {
            (2.0 * c * rho / (c * c - 1.0)).atan() / 2.0
        };
        let alpha = alpha.rem_euclid(std::f64::consts::FRAC_PI_2);
        let mu = (1.0 + (4.0 * rho * rho - 4.0) / (c + 1.0 / c).powi(2)).clamp(0.0, 1.0);
        let gamma = ((1.0 - mu.sqrt()) / (1.0 + mu.sqrt())).sqrt().atan();
        let (sa, ca) = alpha.sin_cos();
        let (sg, cg) = gamma.sin_cos();
        [
            root * ca * cg,
            root * sa * cg,
            -root * sa * sg,
            root * ca * sg,
        ]
    };
    Ok(Matrix { h11, h12, h21, h22 })
}
/// Common geometry is already validated/mapped before deriving any matrix.
pub fn envelope(parameters: &Indices) -> Result<Vec<Matrix>> {
    parameters.validate()?;
    parameters
        .iid
        .iter()
        .zip(&parameters.icc)
        .map(|(&iid, &icc)| matrix(parameters.iid_mode, iid, parameters.icc_mode, icc))
        .collect()
}
/// Ordered h11/h12/h21/h22 after OPD/IPD rotation and optional conjugation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ComplexMatrix {
    pub h11: Complex,
    pub h12: Complex,
    pub h21: Complex,
    pub h22: Complex,
}
impl ComplexMatrix {
    /// Left=h11*mono+h21*decorrelated; right=h12*mono+h22*decorrelated.
    /// Slot interpolation and hybrid routing are supplied by the DSP owner.
    pub fn apply(self, mono: Complex, decorrelated: Complex) -> Result<[Complex; 2]> {
        if self
            .coefficients()
            .iter()
            .chain([mono, decorrelated].iter())
            .any(|x| !x.re.is_finite() || !x.im.is_finite())
        {
            return Err(invalid("invalid PS complex mixing input"));
        }
        let mul = |a: Complex, b: Complex| Complex {
            re: a.re * b.re - a.im * b.im,
            im: a.re * b.im + a.im * b.re,
        };
        let channel = |direct: Complex, diffuse: Complex| {
            let a = mul(direct, mono);
            let b = mul(diffuse, decorrelated);
            Complex {
                re: a.re + b.re,
                im: a.im + b.im,
            }
        };
        let output = [channel(self.h11, self.h21), channel(self.h12, self.h22)];
        if output
            .iter()
            .any(|x| !x.re.is_finite() || !x.im.is_finite())
        {
            return Err(invalid("unrepresentable PS complex mixing output"));
        }
        Ok(output)
    }
    pub fn coefficients(self) -> [Complex; 4] {
        [self.h11, self.h12, self.h21, self.h22]
    }
}
fn smoothed(indices: [i16; 3]) -> Result<Complex> {
    let mut re = 0.0;
    let mut im = 0.0;
    for (index, weight) in indices.into_iter().zip([0.25, 0.5, 1.0]) {
        let phase = aac_ps_dequant::phase_radians(index)?;
        let (sn, cs) = phase.sin_cos();
        re += weight * cs;
        im += weight * sn;
    }
    // The current unit phasor outweighs the two historic terms (1 > 3/4),
    // so the norm is at least 1/4; no invented epsilon or angle fallback.
    let length = re.hypot(im);
    Ok(Complex {
        re: re / length,
        im: im / length,
    })
}
/// Histories are oldest, previous, current, mapped to the CURRENT parameter
/// geometry by their owner. Startup uses index zero for the older two sets.
/// A starred hybrid binding conjugates all four resulting coefficients.
pub fn phase_matrix(
    real: Matrix,
    ipd: [i16; 3],
    opd: [i16; 3],
    conjugate: bool,
) -> Result<ComplexMatrix> {
    if real.coefficients().iter().any(|v| !v.is_finite()) {
        return Err(invalid("invalid PS real matrix for phase rotation"));
    }
    let i = smoothed(ipd)?;
    let o = smoothed(opd)?;
    let right = Complex {
        re: o.re * i.re + o.im * i.im,
        im: o.im * i.re - o.re * i.im,
    };
    let rotated = |h: f64, phase: Complex| Complex {
        re: h * phase.re,
        im: h * phase.im * if conjugate { -1.0 } else { 1.0 },
    };
    let result = ComplexMatrix {
        h11: rotated(real.h11, o),
        h12: rotated(real.h12, right),
        h21: rotated(real.h21, o),
        h22: rotated(real.h22, right),
    };
    if result
        .coefficients()
        .iter()
        .any(|v| !v.re.is_finite() || !v.im.is_finite())
    {
        return Err(invalid("unrepresentable PS rotated matrix"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_phase_history_matches_independent_decimal_rotations_and_conjugation() {
        let g: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ps-mixing-oracles.json"
        ))
        .unwrap();
        let real = matrix(IidMode::new(0).unwrap(), 2, IccMode::new(0).unwrap(), 4).unwrap();
        for row in g["phase"].as_array().unwrap() {
            let values = |key: &str| std::array::from_fn(|i| row[key][i].as_i64().unwrap() as i16);
            let normal = phase_matrix(real, values("ipd"), values("opd"), false).unwrap();
            let conjugated = phase_matrix(real, values("ipd"), values("opd"), true).unwrap();
            let outputs = normal
                .apply(Complex { re: 0.3, im: -0.7 }, Complex { re: 0.4, im: 0.2 })
                .unwrap();
            for (a, e) in outputs.iter().zip(row["outputs"].as_array().unwrap()) {
                assert!((a.re - e[0].as_str().unwrap().parse::<f64>().unwrap()).abs() < 4e-14);
                assert!((a.im - e[1].as_str().unwrap().parse::<f64>().unwrap()).abs() < 4e-14);
            }
            for ((a, c), e) in normal
                .coefficients()
                .iter()
                .zip(conjugated.coefficients())
                .zip(row["expected"].as_array().unwrap())
            {
                let expected = |i: usize| e[i].as_str().unwrap().parse::<f64>().unwrap();
                assert!((a.re - expected(0)).abs() < 3e-14);
                assert!((a.im - expected(1)).abs() < 3e-14);
                assert_eq!(c.re, a.re);
                assert_eq!(c.im, -a.im);
            }
        }
        assert!(phase_matrix(real, [0, 0, 8], [0; 3], false).is_err());
        let rotated = phase_matrix(real, [0; 3], [0; 3], false).unwrap();
        assert!(
            rotated
                .apply(
                    Complex {
                        re: f64::NAN,
                        im: 0.0
                    },
                    Complex::default()
                )
                .is_err()
        );
        let huge = ComplexMatrix {
            h11: Complex {
                re: f64::MAX,
                im: 0.0,
            },
            ..ComplexMatrix::default()
        };
        assert!(
            huge.apply(Complex { re: 2.0, im: 0.0 }, Complex::default())
                .is_err()
        );

        assert!(
            phase_matrix(
                Matrix {
                    h11: f64::NAN,
                    ..real
                },
                [0; 3],
                [0; 3],
                false
            )
            .is_err()
        );
    }
    #[test]
    fn every_quantized_matrix_matches_independent_decimal_algebraic_oracle() {
        let g: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ps-mixing-oracles.json"
        ))
        .unwrap();
        for row in g["rows"].as_array().unwrap() {
            let iid = IidMode::new(if row["fine"].as_bool().unwrap() { 3 } else { 0 }).unwrap();
            let icc = IccMode::new(if row["mode"] == "b" { 3 } else { 0 }).unwrap();
            let m = matrix(
                iid,
                row["iid"].as_i64().unwrap() as i16,
                icc,
                row["icc"].as_i64().unwrap() as i16,
            )
            .unwrap();
            for (a, e) in m
                .coefficients()
                .iter()
                .zip(row["expected"].as_array().unwrap())
            {
                let e = e.as_str().unwrap().parse::<f64>().unwrap();
                assert!((a - e).abs() < 2e-11, "{row}: {a} != {e}");
            }
            assert!(m.coefficients().iter().all(|x| x.is_finite()));
            let left = m.h11 * m.h11 + m.h21 * m.h21;
            let right = m.h12 * m.h12 + m.h22 * m.h22;
            assert!((left + right - 2.0).abs() < 2e-14);
            let db = aac_ps_dequant::iid_db(iid, row["iid"].as_i64().unwrap() as i16).unwrap();
            assert!((10.0 * (left / right).log10() - db).abs() < 2e-9);
            let coherence = (m.h11 * m.h12 + m.h21 * m.h22) / (left * right).sqrt();
            let rho = aac_ps_dequant::coherence(row["icc"].as_i64().unwrap() as i16).unwrap();
            let expected = if icc.mixing_b() { rho.max(0.05) } else { rho };
            assert!((coherence - expected).abs() < 2e-11);
        }
    }
    #[test]
    fn grids_refuse_invalid_indices_and_mode_b_clamps_only_coherence() {
        let iid = IidMode::new(0).unwrap();
        let a = IccMode::new(0).unwrap();
        let b = IccMode::new(3).unwrap();
        assert!(matrix(iid, 8, a, 0).is_err());
        assert!(matrix(iid, 0, a, 8).is_err());
        assert_eq!(matrix(iid, 0, b, 5).unwrap(), matrix(iid, 0, b, 7).unwrap());
        assert_ne!(matrix(iid, 0, a, 5).unwrap(), matrix(iid, 0, a, 7).unwrap());
        assert_eq!(
            matrix(iid, 0, a, 0).unwrap().coefficients(),
            [1.0, 1.0, 0.0, 0.0]
        );
    }
}
