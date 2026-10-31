//! Ordinary full-output TTT trees 2/3/4, composed from shared OTT basis nodes.
//! Arbitrary-downmix gains enter before inversion;HRTF/3D stereo are separate.
use super::{
    Result,
    aac_ld_mps_ott::Ott,
    aac_ld_mps_tree_matrix::{Band, Composer, Matrices as RealMatrices},
    aac_ld_mps_ttt,
    aac_sbr_qmf::Complex,
    invalid, unsupported,
};
#[derive(Clone, Copy, Debug)]
pub struct Ttt {
    pub mode: u8,
    /// Physical CPC for modes 0/1, CLD dB for modes 2..5.
    pub parameters: [f64; 2],
    pub coherence: f64,
    pub residual: bool,
    pub inversion: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Auxiliary {
    Ott(usize),
    Ttt,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Matrices {
    /// Input columns are downmix L/R and the decoded TTT residual basis.
    pub m1: Vec<Vec<Complex>>,
    /// W inputs start with virtual L/R/C, then auxiliaries in declared order.
    pub dry: Vec<Vec<f64>>,
    pub wet: Vec<Vec<f64>>,
    pub auxiliaries: Vec<Auxiliary>,
}
/// One prepared band. Tree2 output order LF/LS/RF/RS/C/LFE; tree3
/// LF/LC/LS/RF/RC/RS/C/LFE; tree4 LF/LS1/LS2/RF/RS1/RS2/C/LFE.
/// Fixed channel gains and optional rendering tools are applied elsewhere.
pub fn build(tree: u8, bands: &[Band], ttt: Ttt, lfe_active: bool) -> Result<Matrices> {
    build_with_downmix(tree, bands, ttt, lfe_active, [1.0, 1.0])
}
/// Linear L/R compensation gains scale the TTT input basis before inversion.
/// Scaling final M1 columns instead would also scale the inverse's cross terms.
pub fn build_with_downmix(
    tree: u8,
    bands: &[Band],
    ttt: Ttt,
    lfe_active: bool,
    downmix: [f64; 2],
) -> Result<Matrices> {
    if downmix.iter().any(|gain| !gain.is_finite() || *gain < 0.0) {
        return Err(invalid("invalid LD-MPS TTT downmix gain"));
    }
    let boxes = match tree {
        2 => 3,
        3 | 4 => 5,
        _ => return Err(invalid("invalid LD-MPS TTT matrix tree")),
    };
    if bands.len() != boxes {
        return Err(invalid("invalid LD-MPS TTT tree OTT count"));
    }
    if bands[0].residual {
        return Err(unsupported(
            "LD-MPS LFE residual matrix routing is not yet qualified",
        ));
    }
    for b in bands {
        Ott::physical(b.cld_db, b.coherence, b.residual)?;
    }
    let mut raw = aac_ld_mps_ttt::transform(
        ttt.mode,
        ttt.parameters[0],
        ttt.parameters[1],
        ttt.coherence,
        ttt.residual,
    )?;
    for row in &mut raw {
        row[0] *= downmix[0];
        row[1] *= downmix[1];
    }
    let mut base = raw.map(|row| row.map(|re| Complex { re, im: 0.0 }));
    // Odd energy modes deliberately bypass the signalled inversion tool.
    if ttt.inversion && (ttt.mode < 2 || ttt.mode == 2 || ttt.mode == 4) {
        let weights = aac_ld_mps_ttt::inversion_weights(
            ttt.mode,
            [bands[1].cld_db, bands[2].cld_db],
            ttt.parameters,
        )?;
        let h = aac_ld_mps_ttt::invert(weights)?.inverse;
        for r in 0..3 {
            for col in 0..2 {
                base[r][col] = Complex {
                    re: raw[r][0] * h[0][col].re + raw[r][1] * h[1][col].re,
                    im: raw[r][0] * h[0][col].im + raw[r][1] * h[1][col].im,
                };
            }
        }
    }
    let aux = if tree == 2 {
        vec![1, 2, boxes]
    } else {
        vec![1, 2, boxes, 3, 4]
    };
    let mut feeds = vec![vec![0.0; 3]; 3 + aux.len()];
    for (i, row) in feeds.iter_mut().take(3).enumerate() {
        row[i] = 1.0;
    }
    let mut c = Composer {
        bands,
        inputs: 3,
        matrices: RealMatrices {
            m1: feeds,
            dry: vec![],
            wet: vec![],
            auxiliary_boxes: aux.clone(),
        },
    };
    let left = c.root(0);
    let right = c.root(1);
    let center = c.root(2);
    let [lf, ls] = c.split(left, 1)?;
    let [rf, rs] = c.split(right, 2)?;
    let [center, lfe] = c.lfe(center, 0, lfe_active)?;
    let left_gain =
        Ott::physical(bands[1].cld_db, bands[1].coherence, bands[1].residual)?.channel_gains[0];
    let right_gain =
        Ott::physical(bands[2].cld_db, bands[2].coherence, bands[2].residual)?.channel_gains[0];
    let center_gain = if lfe_active {
        Ott::physical(bands[0].cld_db, bands[0].coherence, false)?.channel_gains[0]
    } else {
        1.0
    };
    // Each tuple identifies one front output and the direct W column whose
    // coherent component is attenuated when the TTT decorrelation basis is used.
    let (output, front) = match tree {
        2 => (
            vec![lf, ls, rf, rs, center, lfe],
            vec![
                (0, 0, left_gain),
                (2, 1, right_gain),
                (4, 2, -2.0f64.sqrt() * center_gain),
            ],
        ),
        3 => {
            let [lf, lc] = c.split(lf, 3)?;
            let [rf, rc] = c.split(rf, 4)?;
            let lh = Ott::physical(bands[3].cld_db, bands[3].coherence, bands[3].residual)?;
            let rh = Ott::physical(bands[4].cld_db, bands[4].coherence, bands[4].residual)?;
            (
                vec![lf, lc, ls, rf, rc, rs, center, lfe],
                vec![
                    (0, 0, left_gain * lh.rows[0][0]),
                    (1, 0, left_gain * lh.rows[1][0]),
                    (3, 1, right_gain * rh.rows[0][0]),
                    (4, 1, right_gain * rh.rows[1][0]),
                    (6, 2, -2.0f64.sqrt() * center_gain),
                ],
            )
        }
        4 => {
            let [ls1, ls2] = c.split(ls, 3)?;
            let [rs1, rs2] = c.split(rs, 4)?;
            (
                vec![lf, ls1, ls2, rf, rs1, rs2, center, lfe],
                vec![
                    (0, 0, left_gain),
                    (3, 1, right_gain),
                    (6, 2, -2.0f64.sqrt() * center_gain),
                ],
            )
        }
        _ => unreachable!(),
    };
    for mut node in output {
        // Dormant direct-column values retained by the reference 525 plane;
        // the W router supplies zero wet samples for those direct columns.
        if tree == 2 {
            node.wet[..3].copy_from_slice(&node.dry[..3]);
        }
        c.matrices.dry.push(node.dry);
        c.matrices.wet.push(node.wet);
    }
    if ttt.mode == 0 {
        c.matrices.m1[5][2] = 1.0;
    }
    if ttt.mode == 0 && !ttt.residual {
        let diffuse = ((1.0 - ttt.coherence * ttt.coherence) / 3.0).sqrt();
        for (row, col, gain) in front {
            c.matrices.wet[row][5] = diffuse * gain;
            if tree == 2 {
                // The 525 dry plane retains this dormant auxiliary value;
                // outside residual bands the W-dry sample itself is zero.
                c.matrices.dry[row][5] = diffuse * gain;
            }
            c.matrices.dry[row][col] *= ttt.coherence;
            if tree == 2 {
                c.matrices.wet[row][col] *= ttt.coherence;
            }
        }
    }
    let m1 = c
        .matrices
        .m1
        .iter()
        .map(|row| {
            (0..3)
                .map(|col| Complex {
                    re: (0..3).map(|k| row[k] * base[k][col].re).sum(),
                    im: (0..3).map(|k| row[k] * base[k][col].im).sum(),
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if m1
        .iter()
        .flatten()
        .any(|v| !v.re.is_finite() || !v.im.is_finite())
    {
        return Err(invalid("overflowing LD-MPS TTT tree matrix"));
    }
    Ok(Matrices {
        m1,
        dry: c.matrices.dry,
        wet: c.matrices.wet,
        auxiliaries: aux
            .into_iter()
            .map(|b| {
                if b == boxes {
                    Auxiliary::Ttt
                } else {
                    Auxiliary::Ott(b)
                }
            })
            .collect(),
    })
}
