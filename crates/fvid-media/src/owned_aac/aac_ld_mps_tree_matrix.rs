//! Symbolic composition of the OTT-only LD-MPS trees. TTT trees and optional
//! arbitrary-downmix/HRTF transforms need separate builders.
use super::{Result, aac_ld_mps_ott::Ott, invalid, unsupported};

#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub cld_db: f64,
    pub coherence: f64,
    pub residual: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Matrices {
    /// Rows feeding direct channels followed by decorrelator channels.
    pub m1: Vec<Vec<f64>>,
    /// Output rows in tree-native order, columns in M1 order. Auxiliary dry
    /// inputs contain decoded residuals, not the M1 decorrelator feeds.
    pub dry: Vec<Vec<f64>>,
    pub wet: Vec<Vec<f64>>,
    /// Box index for each auxiliary column; LFE split has no decorrelator.
    pub auxiliary_boxes: Vec<usize>,
}
#[derive(Clone)]
pub(super) struct Node {
    pub(super) dry: Vec<f64>,
    pub(super) wet: Vec<f64>,
    pub(super) gain: Vec<f64>,
}
pub(super) struct Composer<'a> {
    pub(super) bands: &'a [Band],
    pub(super) matrices: Matrices,
    pub(super) inputs: usize,
}
impl Composer<'_> {
    pub(super) fn root(&self, input: usize) -> Node {
        let mut dry = vec![0.0; self.matrices.m1.len()];
        dry[input] = 1.0;
        Node {
            dry,
            wet: vec![0.0; self.matrices.m1.len()],
            gain: self.matrices.m1[input].clone(),
        }
    }
    pub(super) fn split(&mut self, node: Node, box_index: usize) -> Result<[Node; 2]> {
        let p = self.bands[box_index];
        let h = Ott::physical(p.cld_db, p.coherence, p.residual)?;
        let col = self.inputs
            + self
                .matrices
                .auxiliary_boxes
                .iter()
                .position(|&b| b == box_index)
                .unwrap();
        self.matrices.m1[col] = node.gain.clone();
        Ok(std::array::from_fn(|side| {
            let row = h.rows[side];
            let mut child = Node {
                dry: node.dry.iter().map(|v| v * row[0]).collect(),
                wet: node.wet.iter().map(|v| v * row[0]).collect(),
                gain: node
                    .gain
                    .iter()
                    .map(|v| v * h.channel_gains[side])
                    .collect(),
            };
            child.dry[col] += row[2];
            child.wet[col] += row[1];
            child
        }))
    }
    pub(super) fn lfe(&self, node: Node, box_index: usize, active: bool) -> Result<[Node; 2]> {
        let gains = if active {
            let p = self.bands[box_index];
            Ott::physical(p.cld_db, p.coherence, false)?.channel_gains
        } else {
            [1.0, 0.0]
        };
        let mut center = node.clone();
        for v in &mut center.dry {
            *v *= gains[0];
        }
        for v in &mut center.wet {
            *v *= gains[0];
        }
        for v in &mut center.gain {
            *v *= gains[0];
        }
        let mut lfe = node;
        lfe.dry.fill(0.0);
        lfe.wet.fill(0.0);
        for (i, v) in lfe.gain.iter_mut().enumerate() {
            *v *= gains[1];
            lfe.dry[i] = *v;
        }
        Ok([center, lfe])
    }
}

/// One physical parameter band, after dequantization/energy adjustment.
/// `lfe_active` selects the CLD-controlled LFE region; above its cutoff the
/// center receives unity and LFE is silent. No fixed output gains are applied.
/// Native output order: tree0 LF/RF/C/LFE/LS/RS; tree1 LF/LS/RF/RS/C/LFE;
/// trees5/6 LF/LS1/LS2/RF/RS1/RS2/C/LFE; tree7 L/R.
pub fn build(tree: u8, bands: &[Band], lfe_active: bool) -> Result<Matrices> {
    let (inputs, boxes, auxiliary): (usize, usize, &[usize]) = match tree {
        0 => (1, 5, &[0, 1, 3, 2]),
        1 => (1, 5, &[0, 1, 3, 4]),
        5 | 6 => (6, 2, &[0, 1]),
        7 => (1, 1, &[0]),
        2..=4 => {
            return Err(unsupported(
                "LD-MPS TTT trees require the TTT matrix builder",
            ));
        }
        _ => return Err(invalid("invalid LD-MPS matrix tree")),
    };
    if bands.len() != boxes {
        return Err(invalid("invalid LD-MPS OTT matrix box count"));
    }
    if matches!(tree, 0 | 1) && bands[if tree == 0 { 4 } else { 2 }].residual {
        return Err(unsupported(
            "LD-MPS LFE residual matrix routing is not yet qualified",
        ));
    }
    for p in bands {
        Ott::physical(p.cld_db, p.coherence, p.residual)?;
    }
    let mut m1 = vec![vec![0.0; inputs]; inputs + auxiliary.len()];
    for (i, row) in m1.iter_mut().take(inputs).enumerate() {
        row[i] = 1.0;
    }
    let mut c = Composer {
        bands,
        inputs,
        matrices: Matrices {
            m1,
            dry: vec![],
            wet: vec![],
            auxiliary_boxes: auxiliary.to_vec(),
        },
    };
    let output = match tree {
        0 => {
            let root = c.root(0);
            let [front, surround] = c.split(root, 0)?;
            let [lr, clfe] = c.split(front, 1)?;
            let [lf, rf] = c.split(lr, 3)?;
            let [ls, rs] = c.split(surround, 2)?;
            let [center, lfe] = c.lfe(clfe, 4, lfe_active)?;
            vec![lf, rf, center, lfe, ls, rs]
        }
        1 => {
            let root = c.root(0);
            let [lr, clfe] = c.split(root, 0)?;
            let [left, right] = c.split(lr, 1)?;
            let [lf, ls] = c.split(left, 3)?;
            let [rf, rs] = c.split(right, 4)?;
            let [center, lfe] = c.lfe(clfe, 2, lfe_active)?;
            vec![lf, ls, rf, rs, center, lfe]
        }
        5 => {
            let left = c.root(0);
            let right = c.root(1);
            let [lf, ls] = c.split(left, 0)?;
            let [rf, rs] = c.split(right, 1)?;
            vec![lf, ls, c.root(4), rf, rs, c.root(5), c.root(2), c.root(3)]
        }
        6 => {
            let left = c.root(4);
            let right = c.root(5);
            let [ls2, ls1] = c.split(left, 0)?;
            let [rs2, rs1] = c.split(right, 1)?;
            vec![
                c.root(0),
                ls1,
                ls2,
                c.root(1),
                rs1,
                rs2,
                c.root(2),
                c.root(3),
            ]
        }
        7 => {
            let root = c.root(0);
            c.split(root, 0)?.to_vec()
        }
        _ => unreachable!(),
    };
    for mut n in output {
        // Mono-tree wet matrices retain their direct column, as specified;
        // the W router supplies zero wet signal in direct columns.
        if inputs == 1 {
            n.wet[0] = n.dry[0];
        }
        c.matrices.dry.push(n.dry);
        c.matrices.wet.push(n.wet);
    }
    Ok(c.matrices)
}
