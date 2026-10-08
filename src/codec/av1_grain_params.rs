//! Owned AV1 film-grain parameter syntax, derived from the AV1 specification.
use super::super::{av1_sequence::Sequence, bits::BitReader};
use crate::{Result, invalid};

/// Fixed-size storage bounds all signaled point and autoregression counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grain {
    pub seed: u16,
    /// Reference slot used by update_grain=0, for syntax qualification.
    pub reference: Option<u8>,
    pub points: [[[u8; 2]; 14]; 3],
    pub point_counts: [usize; 3],
    pub chroma_from_luma: bool,
    pub scaling_shift: u8,
    pub ar_lag: usize,
    pub ar: [[i16; 25]; 3],
    pub ar_shift: u8,
    pub grain_shift: u8,
    pub chroma_mult: [u8; 2],
    pub luma_mult: [u8; 2],
    pub chroma_offset: [u16; 2],
    pub overlap: bool,
    pub restricted_range: bool,
}
fn points(b: &mut BitReader<'_>, g: &mut Grain, p: usize) -> Result<()> {
    let count = b.read(4)? as usize;
    if count > if p == 0 { 14 } else { 10 } {
        return Err(invalid("AV1 film grain point count exceeds limit"));
    }
    g.point_counts[p] = count;
    for i in 0..count {
        let value = b.read(8)? as u8;
        let scale = b.read(8)? as u8;
        if i > 0 && value <= g.points[p][i - 1][0] {
            return Err(invalid("AV1 film grain points are not increasing"));
        }
        g.points[p][i] = [value, scale];
    }
    Ok(())
}
impl Grain {
    pub(super) fn parse(
        b: &mut BitReader<'_>,
        s: &Sequence,
        kind: u8,
        visible: bool,
        references: [usize; 7],
        saved: [Option<Option<&Grain>>; 8],
    ) -> Result<Option<Self>> {
        if !s.film_grain || !visible || !b.bit()? {
            return Ok(None);
        }
        let seed = b.read(16)? as u16;
        if kind == 1 && !b.bit()? {
            let index = b.read(3)? as usize;
            if !references.contains(&index) {
                return Err(invalid("AV1 film grain reference is not a frame reference"));
            }
            let stored =
                saved[index].ok_or_else(|| invalid("AV1 film grain reference is unavailable"))?;
            // reset_grain_params from an apply_grain=0 reference is also inherited.
            let Some(stored) = stored else {
                return Ok(None);
            };
            let mut grain = stored.clone();
            grain.seed = seed;
            grain.reference = Some(index as u8);
            return Ok(Some(grain));
        }
        let mut g = Self {
            seed,
            ..Self::default()
        };
        points(b, &mut g, 0)?;
        g.chroma_from_luma = !s.color.monochrome && b.bit()?;
        if !s.color.monochrome
            && !g.chroma_from_luma
            && !(s.color.subsampling == [true, true] && g.point_counts[0] == 0)
        {
            points(b, &mut g, 1)?;
            points(b, &mut g, 2)?;
            if s.color.subsampling == [true, true]
                && g.point_counts[1] == 0
                && g.point_counts[2] != 0
            {
                return Err(invalid("AV1 film grain 4:2:0 chroma point counts disagree"));
            }
        }
        g.scaling_shift = b.read(2)? as u8 + 8;
        g.ar_lag = b.read(2)? as usize;
        let luma_count = 2 * g.ar_lag * (g.ar_lag + 1);
        for p in 0..3 {
            let count = if p == 0 {
                if g.point_counts[0] == 0 {
                    0
                } else {
                    luma_count
                }
            } else if g.chroma_from_luma || g.point_counts[p] != 0 {
                luma_count + usize::from(g.point_counts[0] != 0)
            } else {
                0
            };
            for v in &mut g.ar[p][..count] {
                *v = b.read(8)? as i16 - 128;
            }
        }
        g.ar_shift = b.read(2)? as u8 + 6;
        g.grain_shift = b.read(2)? as u8;
        for p in 0..2 {
            if g.point_counts[p + 1] != 0 {
                g.chroma_mult[p] = b.read(8)? as u8;
                g.luma_mult[p] = b.read(8)? as u8;
                g.chroma_offset[p] = b.read(9)? as u16;
            }
        }
        g.overlap = b.bit()?;
        g.restricted_range = b.bit()?;
        Ok(Some(g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sequence() -> Sequence {
        let obus = super::super::super::av1::Obus::new(include_bytes!(
            "../../tests/fixtures/av1/sequence.obu"
        ));
        Sequence::parse(
            obus.map(Result::unwrap)
                .find(|o| o.kind == 1)
                .unwrap()
                .payload,
        )
        .unwrap()
    }
    fn bits(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut n = 0;
        for &(value, width) in fields {
            for i in (0..width).rev() {
                if n % 8 == 0 {
                    out.push(0);
                }
                let last = out.len() - 1;
                out[last] |= (((value >> i) & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        out
    }
    #[test]
    fn inherited_parameters_keep_all_fields_but_replace_seed() {
        let mut s = sequence();
        s.film_grain = true;
        let grain = Grain {
            seed: 7,
            point_counts: [14, 10, 10],
            ar_lag: 3,
            overlap: true,
            restricted_range: true,
            ar: [[-127; 25]; 3],
            ..Grain::default()
        };
        let mut saved = [None; 8];
        saved[5] = Some(Some(&grain));
        let data = bits(&[(1, 1), (65534, 16), (0, 1), (5, 3)]);
        let mut reader = BitReader::new(&data);
        let parsed = Grain::parse(&mut reader, &s, 1, true, [5; 7], saved)
            .unwrap()
            .unwrap();
        let mut reset_saved = saved;
        reset_saved[5] = Some(None);
        assert!(
            Grain::parse(&mut BitReader::new(&data), &s, 1, true, [5; 7], reset_saved)
                .unwrap()
                .is_none()
        );
        let mut expected = grain.clone();
        expected.seed = 65534;
        expected.reference = Some(5);
        assert_eq!(parsed, expected);
        assert_eq!(reader.position(), 21);
        assert!(Grain::parse(&mut BitReader::new(&data), &s, 1, true, [4; 7], saved).is_err());
        assert!(Grain::parse(&mut BitReader::new(&data), &s, 1, true, [5; 7], [None; 8]).is_err());
    }
    #[test]
    fn point_counts_and_order_are_validated_before_storage() {
        for p in 0..3 {
            let data = bits(&[(if p == 0 { 15 } else { 11 }, 4)]);
            assert!(points(&mut BitReader::new(&data), &mut Grain::default(), p).is_err());
            for second in [9, 10] {
                let data = bits(&[(2, 4), (10, 8), (20, 8), (second, 8), (30, 8)]);
                assert!(points(&mut BitReader::new(&data), &mut Grain::default(), p).is_err());
            }
        }
    }
    #[test]
    fn maximum_points_and_ar_counts_consume_exact_syntax() {
        let mut s = sequence();
        s.film_grain = true;
        for monochrome in [false, true] {
            s.color.monochrome = monochrome;
            let mut fields = vec![(1, 1), (1234, 16), (14, 4)];
            for i in 0..14 {
                fields.extend([(i * 18, 8), (255 - i, 8)]);
            }
            if !monochrome {
                fields.push((0, 1));
                for _ in 0..2 {
                    fields.push((10, 4));
                    for i in 0..10 {
                        fields.extend([(i * 25, 8), (i * 3, 8)]);
                    }
                }
            }
            fields.extend([(3, 2), (3, 2)]);
            for p in 0..if monochrome { 1 } else { 3 } {
                for i in 0..if p == 0 { 24 } else { 25 } {
                    fields.push((i * 10, 8));
                }
            }
            fields.extend([(3, 2), (2, 2)]);
            if !monochrome {
                for _ in 0..2 {
                    fields.extend([(255, 8), (254, 8), (511, 9)]);
                }
            }
            fields.extend([(1, 1), (1, 1)]);
            let expected_bits: usize = fields.iter().map(|f| f.1 as usize).sum();
            let bytes = bits(&fields);
            let mut reader = BitReader::new(&bytes);
            let g = Grain::parse(&mut reader, &s, 0, true, [0; 7], [None; 8])
                .unwrap()
                .unwrap();
            assert_eq!(reader.position(), expected_bits);
            assert_eq!(g.point_counts[0], 14);
            assert_eq!(g.ar[0][23], 102);
            assert_eq!(g.scaling_shift, 11);
            assert_eq!(g.ar_shift, 9);
            if !monochrome {
                assert_eq!(g.point_counts[1], 10);
                assert_eq!(g.ar[2][24], 112);
                assert_eq!(g.chroma_offset, [511; 2]);
            } else {
                assert_eq!(g.point_counts[1..], [0, 0]);
            }
        }
    }
    #[test]
    fn absent_grain_does_not_consume_unrelated_bits() {
        let mut s = sequence();
        s.film_grain = false;
        let data = [0xff];
        let mut b = BitReader::new(&data);
        assert!(
            Grain::parse(&mut b, &s, 0, true, [0; 7], [None; 8])
                .unwrap()
                .is_none()
        );
        assert_eq!(b.position(), 0);
        s.film_grain = true;
        assert!(
            Grain::parse(&mut b, &s, 0, false, [0; 7], [None; 8])
                .unwrap()
                .is_none()
        );
        assert_eq!(b.position(), 0);
        let mut b = BitReader::new(&[0]);
        assert!(
            Grain::parse(&mut b, &s, 0, true, [0; 7], [None; 8])
                .unwrap()
                .is_none()
        );
        assert_eq!(b.position(), 1);
    }
}
