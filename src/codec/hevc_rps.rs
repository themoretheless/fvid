//! H.265 7.3.7/7.4.8 short-term reference picture set syntax and derivation.
use super::bits::BitReader;
use crate::{Result, invalid};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortTermReference {
    pub delta_poc: i32,
    pub used: bool,
}
fn ue(b: &mut BitReader<'_>, max: u32) -> Result<u32> {
    let n = b.unsigned_golomb()?;
    if n > max {
        return Err(invalid("HEVC reference set value exceeds range"));
    }
    Ok(n)
}
/// Entries are negative POC differences nearest first, then positive nearest first.
/// For SPS syntax `previous` contains all earlier sets. For a slice-local set it
/// contains all SPS sets and `in_slice` enables the explicit predictor distance.
pub fn read_short_term(
    b: &mut BitReader<'_>,
    previous: &[Vec<ShortTermReference>],
    in_slice: bool,
    max_references: u8,
) -> Result<Vec<ShortTermReference>> {
    Ok(read_short_term_syntax(b, previous, in_slice, max_references)?.references)
}

/// Syntax accounting needed by hardware picture submission, before derivation
/// removes zero POC entries or excluded references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortTermSyntax {
    pub references: Vec<ShortTermReference>,
    pub bit_length: usize,
    pub predictor_delta_pocs: usize,
}

pub fn read_short_term_syntax(
    b: &mut BitReader<'_>,
    previous: &[Vec<ShortTermReference>],
    in_slice: bool,
    max_references: u8,
) -> Result<ShortTermSyntax> {
    let start = b.position();
    let mut predictor_delta_pocs = 0;
    if previous.len() > 64 || max_references > 15 {
        return Err(invalid("HEVC reference set configuration exceeds limits"));
    }
    let mut result = Vec::with_capacity(usize::from(max_references));
    if !previous.is_empty() && b.bit()? {
        let distance = if in_slice {
            ue(b, previous.len() as u32 - 1)? as usize + 1
        } else {
            1
        };
        let source = &previous[previous.len() - distance];
        predictor_delta_pocs = source.len();
        if source.len() > usize::from(max_references) {
            return Err(invalid("HEVC RPS predictor exceeds DPB"));
        }
        let mut last_negative = 0;
        let mut last_positive = 0;
        for r in source {
            if r.delta_poc < 0 && last_positive == 0 && r.delta_poc < last_negative {
                last_negative = r.delta_poc;
            } else if r.delta_poc > last_positive {
                last_positive = r.delta_poc;
            } else {
                return Err(invalid("noncanonical HEVC RPS predictor"));
            }
        }
        let sign = if b.bit()? { -1 } else { 1 };
        let delta = sign * (ue(b, 32767)? as i32 + 1);
        for index in 0..=source.len() {
            let used = b.bit()?;
            let include = used || b.bit()?;
            if include {
                let original = source.get(index).map_or(0, |r| r.delta_poc);
                let delta_poc = original
                    .checked_add(delta)
                    .ok_or_else(|| invalid("HEVC RPS POC overflow"))?;
                if delta_poc != 0 {
                    result.push(ShortTermReference { delta_poc, used });
                }
            }
        }
        // This is equivalent to the signed traversal in equations 7-61/7-62,
        // retaining each used flag with its shifted source POC.
        result.sort_unstable_by_key(|r| (r.delta_poc > 0, i64::from(r.delta_poc).abs()));
    } else {
        let negative = ue(b, u32::from(max_references))?;
        let positive = ue(b, u32::from(max_references))?;
        if negative + positive > u32::from(max_references) {
            return Err(invalid("HEVC RPS exceeds DPB"));
        }
        for (count, sign) in [(negative, -1i32), (positive, 1)] {
            let mut delta_poc = 0i32;
            for _ in 0..count {
                delta_poc += sign * (ue(b, 32767)? as i32 + 1);
                result.push(ShortTermReference {
                    delta_poc,
                    used: b.bit()?,
                });
            }
        }
    }
    if result.len() > usize::from(max_references) {
        return Err(invalid("derived HEVC RPS exceeds DPB"));
    }
    Ok(ShortTermSyntax {
        references: result,
        bit_length: b.position() - start,
        predictor_delta_pocs,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn bits(text: &str) -> Vec<u8> {
        let mut s = text.to_owned();
        s.push('1');
        while s.len() % 8 != 0 {
            s.push('0');
        }
        s.as_bytes()
            .chunks(8)
            .map(|c| c.iter().fold(0, |v, b| (v << 1) | (b - b'0')))
            .collect()
    }
    fn r(delta_poc: i32, used: bool) -> ShortTermReference {
        ShortTermReference { delta_poc, used }
    }
    #[test]
    fn invalid_predictors_and_derived_capacity_are_rejected() {
        let data = bits("10111"); // predicted delta=+1; source and delta both used
        assert!(
            read_short_term(&mut BitReader::new(&data), &[vec![r(2, true)]], false, 1).is_err()
        );
        assert!(
            read_short_term(
                &mut BitReader::new(&data),
                &[vec![r(i32::MAX, true)]],
                false,
                2
            )
            .is_err()
        );
        for source in [
            vec![r(0, true)],
            vec![r(-2, true), r(-1, true)],
            vec![r(1, true), r(-1, true)],
        ] {
            assert!(read_short_term(&mut BitReader::new(&data), &[source], false, 4).is_err());
        }
        let distance = bits("1010"); // predictor distance two when only one exists
        assert!(read_short_term(&mut BitReader::new(&distance), &[vec![]], true, 4).is_err());
        assert!(read_short_term(&mut BitReader::new(&[]), &[], false, 4).is_err());
        assert!(read_short_term(&mut BitReader::new(&[]), &[], false, 16).is_err());
    }
    #[test]
    fn predicted_set_crosses_zero_preserving_use_flags_and_order() {
        let previous = vec![vec![r(-1, true), r(-3, true), r(2, true), r(5, true)]];
        // Prediction, delta=-2. Flags: used, retained-unused, used, excluded, used.
        let data = bits("110101011001");
        let mut b = BitReader::new(&data);
        let actual = read_short_term_syntax(&mut b, &previous, false, 5).unwrap();
        assert_eq!(
            actual.references,
            vec![r(-2, true), r(-3, true), r(-5, false)]
        );
        assert_eq!(actual.bit_length, 12);
        assert_eq!(actual.predictor_delta_pocs, 4);
        b.finish_rbsp().unwrap();
    }
    #[test]
    fn explicit_set_and_slice_predictor_distance() {
        // negative=1, positive=1; delta=-1 used, delta=2 unused.
        let data = bits("010010110100");
        let mut b = BitReader::new(&data);
        let syntax = read_short_term_syntax(&mut b, &[], false, 4).unwrap();
        assert_eq!(syntax.bit_length, 12);
        assert_eq!(syntax.predictor_delta_pocs, 0);
        let first = syntax.references;
        assert_eq!(first, vec![r(-1, true), r(2, false)]);
        b.finish_rbsp().unwrap();
        // predicted, distance=2, delta=+1, all three entries used.
        let data = bits("101001111");
        let mut b = BitReader::new(&data);
        let actual = read_short_term_syntax(&mut b, &[first, vec![]], true, 4).unwrap();
        assert_eq!(actual.references, vec![r(1, true), r(3, true)]);
        assert_eq!(actual.bit_length, 9);
        assert_eq!(actual.predictor_delta_pocs, 2);
        b.finish_rbsp().unwrap();
    }
}
