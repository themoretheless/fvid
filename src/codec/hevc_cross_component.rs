//! H.265 8.6.6 cross-component residual modification, before sample clipping.
use crate::{Result, invalid};

/// H.265 7.3.8.11: bounded unary magnitude followed by a context-coded sign.
pub fn read_alpha(
    bins: &mut impl super::hevc_residual::ResidualBins,
    component: usize,
) -> Result<i8> {
    use super::hevc_cabac::Syntax;
    if !(1..=2).contains(&component) {
        return Err(invalid("invalid HEVC cross-component colour component"));
    }
    let base = (component - 1) * 4;
    let mut magnitude = 0;
    while magnitude < 4 {
        if !bins.decision(Syntax::CrossComponentMagnitude, base + magnitude)? {
            break;
        }
        magnitude += 1;
    }
    if magnitude == 0 {
        return Ok(0);
    }
    let sign = bins.decision(Syntax::CrossComponentSign, component - 1)?;
    let alpha = 1i8 << (magnitude - 1);
    Ok(if sign { -alpha } else { alpha })
}

/// Apply the signalled power-of-two alpha. Failure leaves chroma unchanged.
pub fn modify(luma: &[i32], chroma: &mut [i32], alpha: i8, depths: [u8; 2]) -> Result<()> {
    if luma.len() != chroma.len()
        || luma.is_empty()
        || !matches!(alpha, -8 | -4 | -2 | -1 | 0 | 1 | 2 | 4 | 8)
        || depths.iter().any(|d| !(8..=16).contains(d))
    {
        return Err(invalid("invalid HEVC cross-component residual parameters"));
    }
    let value = |y: i32, c: i32| {
        let scaled = (i64::from(y) << depths[1]) >> depths[0];
        i32::try_from(i64::from(c) + ((i64::from(alpha) * scaled) >> 3))
            .map_err(|_| invalid("HEVC cross-component residual overflow"))
    };
    for (&y, &c) in luma.iter().zip(chroma.iter()) {
        value(y, c)?;
    }
    for (&y, c) in luma.iter().zip(chroma.iter_mut()) {
        *c = value(y, *c)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_streams_exercise_nonzero_cross_component_prediction() {
        use super::super::hevc_decoder::HevcDecoder;
        use crate::container::mp4::Mp4Reader;
        use std::io::Cursor;
        for source in [
            include_bytes!("../../tests/fixtures/playback-errors/hevc-cross-component-rext8.mp4")
                .as_slice(),
            include_bytes!("../../tests/fixtures/playback-errors/hevc-cross-component-rext10.mp4")
                .as_slice(),
            include_bytes!("../../tests/fixtures/playback-errors/hevc-cross-component-rext12.mp4")
                .as_slice(),
            include_bytes!(
                "../../tests/fixtures/playback-errors/hevc-cross-component-parallel-rext12.mp4"
            )
            .as_slice(),
            include_bytes!(
                "../../tests/fixtures/playback-errors/hevc-mixed-depth-y8-c10-cross.mp4"
            )
            .as_slice(),
            include_bytes!(
                "../../tests/fixtures/playback-errors/hevc-mixed-depth-y10-c8-cross.mp4"
            )
            .as_slice(),
            include_bytes!(
                "../../tests/fixtures/playback-errors/hevc-mixed-depth-y12-c10-cross.mp4"
            )
            .as_slice(),
        ] {
            let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
            let mut decoder =
                HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20)
                    .unwrap();
            let mut active = 0;
            for index in 0..3 {
                let mut packet = Vec::new();
                input.read_packet(0, index, &mut packet).unwrap();
                active += decoder
                    .decode_packet(&packet)
                    .unwrap()
                    .unwrap()
                    .picture
                    .cross_component_blocks;
            }
            assert!(active > 0, "fixture has no nonzero cross-component alpha");
        }
    }
    #[test]
    fn alpha_syntax_uses_component_banks_and_truncated_unary() {
        use super::super::{hevc_cabac::Syntax, hevc_residual::ResidualBins};
        use std::collections::VecDeque;
        struct Bins(VecDeque<(Syntax, usize, bool)>);
        impl ResidualBins for Bins {
            fn decision(&mut self, syntax: Syntax, context: usize) -> Result<bool> {
                let (s, c, v) = self
                    .0
                    .pop_front()
                    .ok_or_else(|| invalid("truncated alpha"))?;
                assert_eq!(std::mem::discriminant(&s), std::mem::discriminant(&syntax));
                assert_eq!(c, context);
                Ok(v)
            }
            fn bypass(&mut self) -> Result<bool> {
                panic!("alpha has no bypass bins")
            }
        }
        for component in [1, 2] {
            for magnitude in 0..=4 {
                for negative in [false, true] {
                    let base = (component - 1) * 4;
                    let mut events = VecDeque::new();
                    for index in 0..magnitude {
                        events.push_back((Syntax::CrossComponentMagnitude, base + index, true));
                    }
                    if magnitude < 4 {
                        events.push_back((
                            Syntax::CrossComponentMagnitude,
                            base + magnitude,
                            false,
                        ));
                    }
                    if magnitude != 0 {
                        events.push_back((Syntax::CrossComponentSign, component - 1, negative));
                    }
                    let mut b = Bins(events.clone());
                    let expected = if magnitude == 0 {
                        0
                    } else {
                        (1i8 << (magnitude - 1)) * if negative { -1 } else { 1 }
                    };
                    assert_eq!(read_alpha(&mut b, component).unwrap(), expected);
                    assert!(b.0.is_empty());
                    for cut in 0..events.len() {
                        let mut b = Bins(events.iter().take(cut).copied().collect());
                        assert!(read_alpha(&mut b, component).is_err());
                    }
                }
            }
        }
        for component in [0, 3, usize::MAX] {
            assert!(read_alpha(&mut Bins(VecDeque::new()), component).is_err());
        }
    }
    #[test]
    fn signed_rounding_and_depth_conversion_match_integer_formula() {
        for depths in [
            [8, 8],
            [10, 10],
            [12, 12],
            [8, 12],
            [12, 8],
            [16, 8],
            [8, 16],
        ] {
            for alpha in [-8, -4, -2, -1, 0, 1, 2, 4, 8] {
                let y: Vec<i32> = (-513..=513).collect();
                let mut c: Vec<i32> = y.iter().map(|v| 17 - v).collect();
                let expected: Vec<i32> = y
                    .iter()
                    .zip(&c)
                    .map(|(&y, &c)| {
                        let scaled =
                            (i128::from(y) * (1i128 << depths[1])).div_euclid(1i128 << depths[0]);
                        (i128::from(c) + (i128::from(alpha) * scaled).div_euclid(8)) as i32
                    })
                    .collect();
                modify(&y, &mut c, alpha, depths).unwrap();
                assert_eq!(c, expected);
            }
        }
        let mut c = [0, 0];
        modify(&[-1, 1], &mut c, 1, [8, 8]).unwrap();
        assert_eq!(c, [-1, 0]);
    }
    #[test]
    fn malformed_parameters_and_late_overflow_are_atomic() {
        for (y, alpha, depths) in [
            ([1, 2], 3, [8, 8]),
            ([1, 2], 1, [7, 8]),
            ([1, i32::MAX], 8, [8, 16]),
        ] {
            let mut c = [5, 7];
            assert!(modify(&y, &mut c, alpha, depths).is_err());
            assert_eq!(c, [5, 7]);
        }
        let mut c = [5, 7];
        assert!(modify(&[1], &mut c, 1, [8, 8]).is_err());
        assert_eq!(c, [5, 7]);
    }
}
