//! Owned TTT band transform and complex downmix inversion primitives.
//! Tree composition, residual decoding and temporal routing remain separate.
use super::{
    Result, aac_ld_mps_entropy::Kind, aac_ld_mps_prepare::dequantize, aac_sbr_qmf::Complex, invalid,
};
fn range(x: f64, lo: f64, hi: f64) -> Result<()> {
    if x.is_finite() && (lo..=hi).contains(&x) {
        Ok(())
    } else {
        Err(invalid("invalid LD-MPS TTT physical parameter"))
    }
}
fn mode_check(mode: u8) -> Result<()> {
    if mode <= 5 {
        Ok(())
    } else {
        Err(invalid("reserved LD-MPS TTT mode"))
    }
}

/// Columns: two downmix channels and the decoded TTT residual basis.
/// Rows: left, right, center (center includes its sqrt(2) normalization).
pub fn from_indices(
    mode: u8,
    first: i16,
    second: i16,
    icc: i16,
    residual: bool,
) -> Result<[[f64; 3]; 3]> {
    mode_check(mode)?;
    let kind = if mode < 2 { Kind::Cpc } else { Kind::Cld };
    transform(
        mode,
        dequantize(kind, first)?,
        dequantize(kind, second)?,
        dequantize(Kind::Icc, icc)?,
        residual,
    )
}
pub fn transform(
    mode: u8,
    first: f64,
    second: f64,
    coherence: f64,
    residual: bool,
) -> Result<[[f64; 3]; 3]> {
    mode_check(mode)?;
    range(coherence, -0.99, 1.0)?;
    let root2 = 2.0f64.sqrt();
    let mut result = if mode < 2 {
        range(first, -2.0, 3.0)?;
        range(second, -2.0, 3.0)?;
        let divisor = if residual { 3.0 } else { 3.0 * coherence };
        if divisor == 0.0 {
            return Err(invalid("singular LD-MPS TTT prediction coherence"));
        }
        [
            [(first + 2.0) / divisor, (second - 1.0) / divisor, 1.0 / 3.0],
            [(first - 1.0) / divisor, (second + 2.0) / divisor, 1.0 / 3.0],
            [
                (1.0 - first) / divisor,
                (1.0 - second) / divisor,
                -1.0 / 3.0,
            ],
        ]
    } else {
        range(first, -150.0, 150.0)?;
        range(second, -150.0, 150.0)?;
        let a = 10.0f64.powf(first / 10.0);
        let b = 10.0f64.powf(second / 10.0);
        // Normalize channel energies first. Conditional covariance gives both
        // the ordinary energy ratios and center-subtraction regression weights.
        let center = 1.0 / (1.0 + a);
        let right = (a / (1.0 + a)) / (1.0 + b);
        let left = right * b;
        let mut rows = [
            [(left / (left + center)).sqrt(), 0.0, 1.0 / 3.0],
            [0.0, (right / (right + center)).sqrt(), 1.0 / 3.0],
            [
                0.5 * (center / (left + center)).sqrt(),
                0.5 * (center / (right + center)).sqrt(),
                -1.0 / 3.0,
            ],
        ];
        if mode < 4 {
            let determinant = left * right + left * center + right * center;
            let wl = (left * (right + center) / determinant).sqrt();
            let wr = (right * (left + center) / determinant).sqrt();
            rows[0][0] = wl;
            rows[0][1] = -center * wl / (right + center);
            rows[1][0] = -center * wr / (left + center);
            rows[1][1] = wr;
        }
        rows
    };
    for v in &mut result[2] {
        *v *= root2;
    }
    if result.iter().flatten().any(|v| !v.is_finite()) {
        return Err(invalid("overflowing LD-MPS TTT transform"));
    }
    Ok(result)
}

/// Inversion weights from OTT CLDs and either CPC prediction or TTT CLD
/// energy parameters. Modes sharing a coding family use the same weights.
pub fn inversion_weights(mode: u8, ott_db: [f64; 2], parameters: [f64; 2]) -> Result<[f64; 2]> {
    mode_check(mode)?;
    for x in ott_db {
        range(x, -150.0, 150.0)?;
    }
    let factors = if mode < 2 {
        for x in parameters {
            range(x, -2.0, 3.0)?;
        }
        parameters.map(|x| {
            if x.abs() >= 1.0 {
                1.0
            } else if x < -0.5 {
                -1.0 - 2.0 * x
            } else {
                (1.0 + 2.0 * x) / 3.0
            }
        })
    } else {
        for x in parameters {
            range(x, -150.0, 150.0)?;
        }
        let a = 10.0f64.powf(parameters[0] / 10.0);
        let b = 10.0f64.powf(parameters[1] / 10.0);
        [
            (a * b / (a * b + 2.0 * (b + 1.0))).sqrt(),
            (a / (a + 2.0 * (b + 1.0))).sqrt(),
        ]
    };
    Ok(std::array::from_fn(|i| {
        factors[i] / (1.0 + 10.0f64.powf(ott_db[i] / 20.0))
    }))
}
fn mul(a: Complex, b: Complex) -> Complex {
    Complex {
        re: a.re * b.re - a.im * b.im,
        im: a.re * b.im + a.im * b.re,
    }
}
fn sub(a: Complex, b: Complex) -> Complex {
    Complex {
        re: a.re - b.re,
        im: a.im - b.im,
    }
}
fn divide(a: Complex, b: Complex) -> Complex {
    let d = b.re * b.re + b.im * b.im;
    Complex {
        re: (a.re * b.re + a.im * b.im) / d,
        im: (a.im * b.re - a.re * b.im) / d,
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Inversion {
    pub forward: [[Complex; 2]; 2],
    pub inverse: [[Complex; 2]; 2],
}
/// Pivoted complex elimination, independent of the reference's explicit
/// determinant/adjugate implementation. Real-only alias-lock DSP is separate.
pub fn invert(weights: [f64; 2]) -> Result<Inversion> {
    for w in weights {
        range(w, 0.0, 1.0)?;
    }
    let [a, b] = weights;
    let na = (1.0 - a).hypot(a);
    let nb = (1.0 - b).hypot(b);
    let root3 = 3.0f64.sqrt();
    let forward = [
        [
            Complex {
                re: (1.0 - a) / na,
                im: a / na,
            },
            Complex {
                re: 0.0,
                im: b / (nb * root3),
            },
        ],
        [
            Complex {
                re: 0.0,
                im: -a / (na * root3),
            },
            Complex {
                re: (1.0 - b) / nb,
                im: -b / nb,
            },
        ],
    ];
    let mut work = [[Complex::default(); 4]; 2];
    for r in 0..2 {
        work[r][..2].copy_from_slice(&forward[r]);
        work[r][2 + r].re = 1.0;
    }
    for col in 0..2 {
        let norm = |z: Complex| z.re * z.re + z.im * z.im;
        if norm(work[1][col]) > norm(work[col][col]) && col == 0 {
            work.swap(0, 1);
        }
        let pivot = work[col][col];
        if norm(pivot) == 0.0 {
            return Err(invalid("singular LD-MPS complex inversion"));
        }
        for j in 0..4 {
            work[col][j] = divide(work[col][j], pivot);
        }
        let other = 1 - col;
        let factor = work[other][col];
        for j in 0..4 {
            work[other][j] = sub(work[other][j], mul(factor, work[col][j]));
        }
    }
    let inverse = std::array::from_fn(|r| [work[r][2], work[r][3]]);
    if inverse
        .iter()
        .flatten()
        .any(|z| !z.re.is_finite() || !z.im.is_finite())
    {
        return Err(invalid("overflowing LD-MPS complex inversion"));
    }
    Ok(Inversion { forward, inverse })
}
