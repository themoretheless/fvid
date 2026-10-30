//! Owned one-to-two LD-MPS matrix from output covariance and mono-sum constraint.
//! Matrix construction/mixing primitive; no residual codec or PCM routing here.
use super::{
    Result, aac_ld_mps_entropy::Kind, aac_ld_mps_prepare::dequantize, aac_sbr_qmf::Complex, invalid,
};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ott {
    /// Output rows [left,right], columns [direct,decorrelated,residual].
    pub rows: [[f64; 3]; 2],
    pub channel_gains: [f64; 2],
    pub effective_coherence: f64,
}
impl Ott {
    pub fn from_indices(cld: i16, icc: i16, residual: bool) -> Result<Self> {
        Self::physical(
            dequantize(Kind::Cld, cld)?,
            dequantize(Kind::Icc, icc)?,
            residual,
        )
    }
    pub fn physical(cld_db: f64, coherence: f64, residual: bool) -> Result<Self> {
        if !cld_db.is_finite()
            || !(-150.0..=150.0).contains(&cld_db)
            || !coherence.is_finite()
            || !(-0.99..=1.0).contains(&coherence)
        {
            return Err(invalid("invalid LD-MPS OTT physical parameters"));
        }
        let ratio = 10.0f64.powf(cld_db / 10.0);
        let right_power = 1.0 / (1.0 + ratio);
        let left_power = ratio * right_power;
        let cross_power = (left_power * right_power).sqrt();
        // The residual branch bounds inverse mono-sum gain by 1.2. With
        // S²=1+2*rho*sqrt(P_l P_r), this is rho >= -11/(72*sqrt(P_l P_r)).
        let rho = if residual {
            coherence.max(-11.0 / (72.0 * cross_power))
        } else {
            coherence
        };
        let sum_gain = (1.0 + 2.0 * rho * cross_power).sqrt();
        let direct_left = (left_power + rho * cross_power) / sum_gain;
        let direct_right = (right_power + rho * cross_power) / sum_gain;
        let diffuse = if residual {
            0.0
        } else {
            cross_power * (1.0 - rho * rho).sqrt() / sum_gain
        };
        Ok(Self {
            rows: [
                [direct_left, diffuse, if residual { 1.0 } else { 0.0 }],
                [direct_right, -diffuse, if residual { -1.0 } else { 0.0 }],
            ],
            channel_gains: [left_power.sqrt(), right_power.sqrt()],
            effective_coherence: rho,
        })
    }
    pub fn apply(
        self,
        direct: Complex,
        decorrelated: Complex,
        residual: Complex,
    ) -> Result<[Complex; 2]> {
        if [direct, decorrelated, residual]
            .iter()
            .any(|x| !x.re.is_finite() || !x.im.is_finite())
            || self.rows.iter().flatten().any(|x| !x.is_finite())
        {
            return Err(invalid("invalid LD-MPS OTT complex input"));
        }
        let input = [direct, decorrelated, residual];
        let out = self.rows.map(|row| Complex {
            re: row.iter().zip(input).map(|(h, x)| h * x.re).sum(),
            im: row.iter().zip(input).map(|(h, x)| h * x.im).sum(),
        });
        if out.iter().any(|x| !x.re.is_finite() || !x.im.is_finite()) {
            return Err(invalid("overflowing LD-MPS OTT complex output"));
        }
        Ok(out)
    }
}
