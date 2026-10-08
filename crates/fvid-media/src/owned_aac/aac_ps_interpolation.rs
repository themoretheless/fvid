//! Owned temporal interpolation of complex PS matrices, normative 6.4.6.4.
//! Coefficients live on the common parameter grid; hybrid binding/conjugation
//! commutes with this linear stage. Configuration remapping is an explicit
//! operation by the owner, not interpolation of old native parameter indices.
use super::{
    Result, aac_ps_mapping::Bands, aac_ps_mixing::ComplexMatrix, aac_sbr_qmf::Complex, invalid,
};
fn valid(matrix: &ComplexMatrix) -> bool {
    matrix
        .coefficients()
        .iter()
        .all(|c| c.re.is_finite() && c.im.is_finite())
}
fn between(a: ComplexMatrix, b: ComplexMatrix, t: f64) -> Result<ComplexMatrix> {
    if t == 0.0 {
        return Ok(a);
    }
    if t == 1.0 {
        return Ok(b);
    }
    // A difference b-a can overflow for opposite finite endpoints. Convex
    // weights avoid that intermediate, without clipping the physical values.
    let scalar = |a: f64, b: f64| {
        if a.is_sign_negative() == b.is_sign_negative() {
            a + (b - a) * t
        } else {
            a * (1.0 - t) + b * t
        }
    };
    let complex = |a: Complex, b: Complex| Complex {
        re: scalar(a.re, b.re),
        im: scalar(a.im, b.im),
    };
    let result = ComplexMatrix {
        h11: complex(a.h11, b.h11),
        h12: complex(a.h12, b.h12),
        h21: complex(a.h21, b.h21),
        h22: complex(a.h22, b.h22),
    };
    if !valid(&result) {
        return Err(invalid("unrepresentable PS interpolated matrix"));
    }
    Ok(result)
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub bands: Bands,
    /// Chronological QMF slots, each containing 20 or 34 parameter matrices.
    pub coefficients: Vec<Vec<ComplexMatrix>>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    bands: Bands,
    previous: Vec<ComplexMatrix>,
}
impl Default for State {
    fn default() -> Self {
        Self::new(Bands::Twenty)
    }
}
impl State {
    pub fn new(bands: Bands) -> Self {
        Self {
            bands,
            previous: vec![ComplexMatrix::default(); bands.count()],
        }
    }
    pub fn bands(&self) -> Bands {
        self.bands
    }
    pub fn previous(&self) -> &[ComplexMatrix] {
        &self.previous
    }
    pub fn reset(&mut self, bands: Bands) {
        *self = Self::new(bands)
    }
    /// The owner maps retained REAL h_ij on a configuration change and
    /// rebuilds complex matrices after resetting phase history. Supply that
    /// explicit common-grid boundary here. No index remapping is inferred.
    pub fn replace_previous(&mut self, bands: Bands, previous: &[ComplexMatrix]) -> Result<()> {
        if previous.len() != bands.count() || previous.iter().any(|m| !valid(m)) {
            return Err(invalid("invalid PS retained interpolation boundary"));
        }
        self.previous = previous.to_vec();
        self.bands = bands;
        Ok(())
    }
    /// Borders are last-slot indices (0 based), not sample counts. A first
    /// border zero applies its endpoint at slot zero. Otherwise the initial
    /// segment uses n/n0 and starts with the previous frame's final matrix.
    /// Between borders use (n-ne)/(ne1-ne); hold the final endpoint to EOF.
    /// Empty endpoints reuse the retained matrices for every slot. Validation
    /// and all interpolation finish before committing the next boundary.
    pub fn process(
        &mut self,
        slots: u8,
        borders: &[u8],
        endpoints: &[Vec<ComplexMatrix>],
    ) -> Result<Frame> {
        if !matches!(slots, 24 | 30 | 32)
            || borders.len() != endpoints.len()
            || endpoints.len() > 4
            || borders.iter().any(|&b| b >= slots)
            || borders.windows(2).any(|b| b[0] >= b[1])
            || endpoints
                .iter()
                .any(|v| v.len() != self.bands.count() || v.iter().any(|m| !valid(m)))
        {
            return Err(invalid("invalid PS interpolation geometry or coefficients"));
        }
        let mut coefficients = Vec::with_capacity(usize::from(slots));
        for n in 0..slots {
            let (a, b, t) = if borders.is_empty() {
                (&self.previous, &self.previous, 0.0)
            } else if n < borders[0] {
                (
                    &self.previous,
                    &endpoints[0],
                    f64::from(n) / f64::from(borders[0]),
                )
            } else if let Some(i) = borders.windows(2).position(|v| n < v[1]) {
                (
                    &endpoints[i],
                    &endpoints[i + 1],
                    f64::from(n - borders[i]) / f64::from(borders[i + 1] - borders[i]),
                )
            } else {
                let last = endpoints.last().unwrap();
                (last, last, 0.0)
            };
            coefficients.push(
                a.iter()
                    .zip(b)
                    .map(|(&a, &b)| between(a, b, t))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        let result = Frame {
            bands: self.bands,
            coefficients,
        };
        if let Some(last) = endpoints.last() {
            self.previous = last.clone()
        }
        Ok(result)
    }
}
