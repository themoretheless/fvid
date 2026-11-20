//! Signalled 3D-stereo HRTF projection and regularized complex inverse.
//! This changes TTT M1;it does not select a two-ear output renderer.
use super::{Result, aac_ld_mps_config::Hrtf, aac_sbr_qmf::Complex, invalid, unsupported};
use std::sync::OnceLock;
type Matrix = [[Complex; 2]; 2];
fn add(a: Complex, b: Complex) -> Complex {
    Complex {
        re: a.re + b.re,
        im: a.im + b.im,
    }
}
fn sub(a: Complex, b: Complex) -> Complex {
    Complex {
        re: a.re - b.re,
        im: a.im - b.im,
    }
}
fn mul(a: Complex, b: Complex) -> Complex {
    Complex {
        re: a.re * b.re - a.im * b.im,
        im: a.re * b.im + a.im * b.re,
    }
}
fn scale(a: Complex, s: f64) -> Complex {
    Complex {
        re: a.re * s,
        im: a.im * s,
    }
}
fn conj(a: Complex) -> Complex {
    Complex {
        re: a.re,
        im: -a.im,
    }
}
fn norm(a: Complex) -> f64 {
    a.re.hypot(a.im)
}
fn polar(r: f64, phase: f64) -> Complex {
    let (im, re) = phase.sin_cos();
    Complex {
        re: r * re,
        im: r * im,
    }
}
fn level(index: u8) -> f64 {
    static LEVELS: OnceLock<[f64; 64]> = OnceLock::new();
    LEVELS.get_or_init(|| std::array::from_fn(|i| 10.0f64.powf((i as f64 - 16.0) / 20.0)))
        [usize::from(index)]
}
fn validate_shape(hrtf: &Hrtf) -> Result<()> {
    if hrtf.channels.len() != 5 || !matches!(hrtf.bands, 4 | 5 | 7 | 9 | 12 | 15 | 23) {
        return Err(invalid("invalid LD-MPS 3D-stereo HRTF geometry"));
    }
    let bands = usize::from(hrtf.bands);
    if hrtf.channels.iter().any(|c| {
        c.left.len() != bands
            || c.right.len() != bands
            || c.phase.as_ref().is_some_and(|v| v.len() != bands)
    }) {
        return Err(invalid("invalid LD-MPS 3D-stereo HRTF geometry"));
    }
    Ok(())
}
pub fn validate(hrtf: &Hrtf, bands: u8) -> Result<()> {
    if hrtf.bands != bands {
        return Err(unsupported(
            "LD-MPS 3D-stereo HRTF grid conversion is not yet qualified",
        ));
    }
    validate_shape(hrtf)?;
    for c in &hrtf.channels {
        if c.left.len() != usize::from(bands)
            || c.right.len() != usize::from(bands)
            || c.phase
                .as_ref()
                .is_some_and(|v| v.len() != usize::from(bands))
            || c.left
                .iter()
                .chain(&c.right)
                .chain(c.phase.iter().flatten())
                .any(|&v| v > 63)
        {
            return Err(invalid("invalid LD-MPS 3D-stereo HRTF values"));
        }
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq)]
pub struct Inversion {
    pub original: Matrix,
    pub forward: Matrix,
    pub inverse: Matrix,
    pub regularized: bool,
}
/// Analytic Hermitian eigenbasis,with a determinant-based small singular value.
/// The determinant floor is0.1. A rank-deficient basis uses an orthogonal
/// completion rather than dividing a zero singular vector by its norm.
pub fn invert(mut h: Matrix) -> Result<Inversion> {
    if h.iter()
        .flatten()
        .any(|v| !v.re.is_finite() || !v.im.is_finite())
    {
        return Err(invalid("nonfinite LD-MPS 3D-stereo matrix"));
    }
    let original = h;
    let det = sub(mul(h[0][0], h[1][1]), mul(h[0][1], h[1][0]));
    let magnitude = norm(det);
    let regularized = magnitude < 0.1;
    let inverse = if !regularized {
        let divisor = scale(conj(det), 1.0 / (magnitude * magnitude));
        [
            [mul(h[1][1], divisor), scale(mul(h[0][1], divisor), -1.)],
            [scale(mul(h[1][0], divisor), -1.), mul(h[0][0], divisor)],
        ]
    } else {
        let a = norm(h[0][0]).powi(2) + norm(h[1][0]).powi(2);
        let c = norm(h[0][1]).powi(2) + norm(h[1][1]).powi(2);
        let b = add(mul(conj(h[0][0]), h[0][1]), mul(conj(h[1][0]), h[1][1]));
        let lambda = (a + c + (a - c).hypot(2. * norm(b))) / 2.;
        let s0 = lambda.sqrt();
        let (v0, u0) = if s0 == 0. {
            (
                [Complex { re: 1., im: 0. }, Complex::default()],
                [Complex { re: 1., im: 0. }, Complex::default()],
            )
        } else {
            let mut v = if norm(b) == 0. {
                if a >= c {
                    [Complex { re: 1., im: 0. }, Complex::default()]
                } else {
                    [Complex::default(), Complex { re: 1., im: 0. }]
                }
            } else {
                [
                    b,
                    Complex {
                        re: lambda - a,
                        im: 0.,
                    },
                ]
            };
            let n = norm(v[0]).hypot(norm(v[1]));
            for z in &mut v {
                *z = scale(*z, 1. / n);
            }
            let mut u = [
                add(mul(h[0][0], v[0]), mul(h[0][1], v[1])),
                add(mul(h[1][0], v[0]), mul(h[1][1], v[1])),
            ];
            let n = norm(u[0]).hypot(norm(u[1]));
            for z in &mut u {
                *z = scale(*z, 1. / n);
            }
            (v, u)
        };
        let v1 = [scale(conj(v0[1]), -1.), conj(v0[0])];
        let phase = if magnitude == 0. {
            Complex { re: 1., im: 0. }
        } else {
            scale(det, 1. / magnitude)
        };
        let u1 = [mul(scale(conj(u0[1]), -1.), phase), mul(conj(u0[0]), phase)];
        let s1 = if s0 == 0. { 0. } else { magnitude / s0 };
        let floor = 0.1f64.sqrt();
        let singular = if s0 < floor && s1 < floor {
            [floor, floor]
        } else {
            [s0, 0.1 / s0]
        };
        let u = [u0, u1];
        let v = [v0, v1];
        h = std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                add(
                    scale(mul(u[0][r], conj(v[0][c])), singular[0]),
                    scale(mul(u[1][r], conj(v[1][c])), singular[1]),
                )
            })
        });
        std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                add(
                    scale(mul(v[0][r], conj(u[0][c])), 1. / singular[0]),
                    scale(mul(v[1][r], conj(u[1][c])), 1. / singular[1]),
                )
            })
        })
    };
    if inverse
        .iter()
        .flatten()
        .chain(h.iter().flatten())
        .any(|v| !v.re.is_finite() || !v.im.is_finite())
    {
        return Err(invalid("overflowing LD-MPS 3D-stereo inverse"));
    }
    Ok(Inversion {
        original,
        forward: h,
        inverse,
        regularized,
    })
}
/// Project the prepared L/R/C TTT basis to the two signalled HRTF responses.
/// Phase is used only below parameter band12;fixed surround gain enters energy.
pub fn project(
    pre: [[f64; 3]; 3],
    ott_db: [f64; 2],
    hrtf: &Hrtf,
    band: usize,
    surround: f64,
) -> Result<Inversion> {
    validate_shape(hrtf)?;
    if band >= usize::from(hrtf.bands)
        || !surround.is_finite()
        || surround < 0.
        || ott_db.iter().any(|v| !v.is_finite() || v.abs() > 150.)
        || pre.iter().flatten().any(|v| !v.is_finite())
    {
        return Err(invalid("invalid LD-MPS 3D-stereo projection"));
    }
    if hrtf.channels.iter().any(|c| {
        c.left[band] > 63 || c.right[band] > 63 || c.phase.as_ref().is_some_and(|v| v[band] > 63)
    }) {
        return Err(invalid("invalid LD-MPS 3D-stereo HRTF values"));
    }
    let responses: [[f64; 3]; 5] = std::array::from_fn(|ch| {
        let c = &hrtf.channels[ch];
        [
            level(c.left[band]),
            level(c.right[band]),
            c.phase
                .as_ref()
                .map_or(0., |v| f64::from(v[band]) * std::f64::consts::PI / 16.),
        ]
    });
    let center = responses[2];
    let pc = [
        polar(center[0], if band < 12 { center[2] / 2. } else { 0. }),
        polar(center[1], if band < 12 { -center[2] / 2. } else { 0. }),
    ];
    let side = |front: usize, rear: usize, cld: f64, right: bool| {
        let ratio = 10.0f64.powf(cld / 10.);
        let weight = ratio / (1. + ratio);
        let f = responses[front];
        let s = responses[rear];
        let energy: [f64; 2] = std::array::from_fn(|ear| {
            weight * f[ear] * f[ear] + (1. - weight) * s[ear] * s[ear] * surround * surround
        });
        let cross = usize::from(!right);
        let phase = if band < 12 {
            (weight * f[cross] * f[cross] * f[2]
                + (1. - weight) * s[cross] * s[cross] * surround * surround * s[2])
                / energy[cross]
                * (if right { 1. } else { -1. })
        } else {
            0.
        };
        if right {
            [polar(energy[0].sqrt(), phase), polar(energy[1].sqrt(), 0.)]
        } else {
            [polar(energy[0].sqrt(), 0.), polar(energy[1].sqrt(), phase)]
        }
    };
    let left = side(0, 3, ott_db[0], false);
    let right = side(1, 4, ott_db[1], true);
    let h = std::array::from_fn(|ear| {
        std::array::from_fn(|col| {
            add(
                add(
                    scale(left[ear], pre[0][col]),
                    scale(right[ear], pre[1][col]),
                ),
                scale(pc[ear], pre[2][col]),
            )
        })
    });
    invert(h)
}
