//! VP9 transform coefficient entropy decoding and dequantization.
use super::{vp9_bool::BoolDecoder, vp9_probs::Probabilities, vp9_tables::*, vp9_transform::Kind};
use crate::{Result, invalid};
const TREE: [i8; 20] = [
    0, 2, -1, 4, 6, 10, -2, 8, -3, -4, 12, 14, -5, -6, 16, 18, -7, -8, -9, -10,
];
const CAT: [&[u8]; 7] = [
    &[],
    &[159],
    &[165, 145],
    &[173, 148, 140],
    &[176, 155, 140, 135],
    &[180, 157, 141, 134, 130],
    &[
        254, 254, 254, 252, 249, 243, 230, 196, 177, 153, 140, 133, 130, 129,
    ],
];
fn scan(size: usize, kind: Kind) -> &'static [u16] {
    match (size, kind) {
        (4, Kind::AdstDct) => &ROW_SCAN_4X4,
        (4, Kind::DctAdst) => &COL_SCAN_4X4,
        (4, _) => &DEFAULT_SCAN_4X4,
        (8, Kind::AdstDct) => &ROW_SCAN_8X8,
        (8, Kind::DctAdst) => &COL_SCAN_8X8,
        (8, _) => &DEFAULT_SCAN_8X8,
        (16, Kind::AdstDct) => &ROW_SCAN_16X16,
        (16, Kind::DctAdst) => &COL_SCAN_16X16,
        (16, _) => &DEFAULT_SCAN_16X16,
        _ => &DEFAULT_SCAN_32X32,
    }
}
fn pareto(node: usize, p: u8) -> Result<u8> {
    if node < 2 {
        return Ok(p);
    }
    if p == 0 {
        return Err(invalid("zero VP9 Pareto probability"));
    }
    let x = usize::from((p - 1) / 2);
    Ok(if p & 1 != 0 {
        PARETO_TABLE[x][node - 2]
    } else {
        ((u16::from(PARETO_TABLE[x][node - 2]) + u16::from(PARETO_TABLE[x + 1][node - 2])) / 2)
            as u8
    })
}
#[derive(Clone, Copy)]
pub struct Config {
    pub size: usize,
    pub kind: Kind,
    pub depth: u8,
    pub chroma: bool,
    pub inter: bool,
    /// Sum of any-nonzero above and left contexts, in 0..=2.
    pub initial_context: usize,
}
pub struct Coefficients {
    pub values: Vec<i32>,
    pub nonzero_context: bool,
}
pub fn read(b: &mut BoolDecoder<'_>, p: &Probabilities, cfg: Config) -> Result<Coefficients> {
    let Config {
        size,
        kind,
        depth,
        chroma,
        inter,
        initial_context,
    } = cfg;
    if ![4, 8, 16, 32].contains(&size)
        || ![8, 10, 12].contains(&depth)
        || initial_context > 2
        || (size == 32 && kind != Kind::DctDct)
    {
        return Err(invalid("invalid VP9 coefficient configuration"));
    }
    let tx = size.trailing_zeros() as usize - 2;
    let scan = scan(size, kind);
    let mut values = vec![0; size * size];
    let mut cache = vec![0usize; size * size];
    let mut check_eob = true;
    let mut count = 0;
    for (c, &position) in scan.iter().enumerate() {
        let pos = usize::from(position);
        let y = pos / size;
        let x = pos % size;
        let band = match c {
            0 => 0,
            1..=2 => 1,
            3..=5 => 2,
            6..=9 => 3,
            10..=12 => 4,
            13..=20 if size > 4 => 4,
            _ => 5,
        };
        let context = if c == 0 {
            initial_context
        } else {
            let (a, d) = if y > 0 && x > 0 {
                match kind {
                    Kind::DctAdst => (pos - size, pos - size),
                    Kind::AdstDct => (pos - 1, pos - 1),
                    _ => (pos - size, pos - 1),
                }
            } else if y > 0 {
                (pos - size, pos - size)
            } else {
                (pos - 1, pos - 1)
            };
            (1 + cache[a] + cache[d]) >> 1
        };
        let probs = p.coef[tx][usize::from(chroma)][usize::from(inter)][band][context];
        if check_eob && !b.read(probs[0])? {
            break;
        }
        let mut node = 0usize;
        let token = loop {
            let prob = pareto(node / 2, probs[(1 + node / 2).min(2)])?;
            let child = TREE[node + usize::from(b.read(prob)?)];
            if child <= 0 {
                break (-child) as usize;
            }
            node = child as usize;
        };
        cache[pos] = [0, 1, 2, 3, 3, 4, 4, 5, 5, 5, 5][token];
        if token == 0 {
            check_eob = false;
        } else {
            let mut coefficient = [0, 1, 2, 3, 4, 5, 7, 11, 19, 35, 67][token];
            if token == 10 {
                for e in 0..depth - 8 {
                    coefficient += i32::from(b.read(255)?) << (5 + depth - e);
                }
            }
            if token >= 5 {
                let cat = CAT[token - 4];
                for (e, &prob) in cat.iter().enumerate() {
                    coefficient += i32::from(b.read(prob)?) << (cat.len() - 1 - e);
                }
            }
            values[pos] = if b.read(128)? {
                -coefficient
            } else {
                coefficient
            };
            check_eob = true;
        }
        count = c + 1;
    }
    Ok(Coefficients {
        values,
        nonzero_context: count > 0,
    })
}
pub fn dequantize(
    values: &[i32],
    size: usize,
    depth: u8,
    q: u8,
    dc_delta: i16,
    ac_delta: i16,
) -> Result<Vec<i32>> {
    if ![4, 8, 16, 32].contains(&size)
        || values.len() != size * size
        || ![8, 10, 12].contains(&depth)
    {
        return Err(invalid("invalid VP9 dequantization configuration"));
    }
    let d = usize::from((depth - 8) / 2);
    let dc = i64::from(DC_QLOOKUP[d][(i32::from(q) + i32::from(dc_delta)).clamp(0, 255) as usize]);
    let ac = i64::from(AC_QLOOKUP[d][(i32::from(q) + i32::from(ac_delta)).clamp(0, 255) as usize]);
    let denom = if size == 32 { 2 } else { 1 };
    let bound = 1i64 << (7 + depth);
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let dequant = i64::from(v) * if i == 0 { dc } else { ac } / denom;
            if !(-bound..bound).contains(&dequant) {
                Err(invalid(
                    "VP9 dequantized coefficient exceeds bit-depth range",
                ))
            } else {
                Ok(dequant as i32)
            }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scans_are_permutations_with_available_context_neighbors() {
        for size in [4, 8, 16, 32] {
            for kind in [Kind::DctDct, Kind::AdstDct, Kind::DctAdst, Kind::AdstAdst] {
                let scan = scan(size, kind);
                let mut seen = vec![false; size * size];
                assert_eq!(scan[0], 0);
                for &pos in scan {
                    let pos = usize::from(pos);
                    assert!(!seen[pos]);
                    seen[pos] = true;
                }
                assert!(seen.iter().all(|v| *v));
            }
        }
    }
    #[test]
    fn zero_eob_and_dequantization_limits() {
        let mut b = BoolDecoder::new(&[0; 16]).unwrap();
        let c = read(
            &mut b,
            &Probabilities::default(),
            Config {
                size: 4,
                kind: Kind::DctDct,
                depth: 8,
                chroma: false,
                inter: false,
                initial_context: 0,
            },
        )
        .unwrap();
        assert!(!c.nonzero_context);
        assert_eq!(c.values, [0; 16]);
        let mut v = vec![0; 16];
        v[0] = 1;
        v[1] = -1;
        assert_eq!(&dequantize(&v, 4, 8, 0, 0, 0).unwrap()[..2], &[4, -4]);
        v[0] = i32::MAX;
        assert!(dequantize(&v, 4, 8, 255, 0, 0).is_err());
    }
}
