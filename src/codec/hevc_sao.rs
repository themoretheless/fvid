//! HEVC CTU sample-adaptive-offset syntax (7.3.8.3).
use super::{hevc_cabac::Syntax, hevc_residual::ResidualBins};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sao {
    #[default]
    Off,
    Band {
        position: u8,
        offsets: [i16; 4],
    },
    /// Classes: horizontal, vertical, down-right, down-left. Offsets are ordered
    /// by edge categories 1..4, with inferred positive/positive/negative/negative.
    Edge {
        class: u8,
        offsets: [i16; 4],
    },
}
pub type CtuSao = [Sao; 3];

impl Sao {
    /// Relative positions of the two edge neighbours, when edge filtering is
    /// selected. Read from the deblocked picture, never already SAO-filtered data.
    pub fn neighbours(self) -> Result<Option<[[i32; 2]; 2]>> {
        match self {
            Self::Edge { class, .. } => Ok(Some(
                *[
                    [[-1, 0], [1, 0]],
                    [[0, -1], [0, 1]],
                    [[-1, -1], [1, 1]],
                    [[1, -1], [-1, 1]],
                ]
                .get(usize::from(class))
                .ok_or_else(|| invalid("invalid SAO edge class"))?,
            )),
            _ => Ok(None),
        }
    }

    /// Apply one Main/Main10 SAO sample. For edge mode, pass None if either
    /// neighbour is unavailable under picture/tile/slice filter boundaries.
    /// PCM/transquant filter exclusions must be handled by the picture caller.
    #[inline]
    pub fn apply(self, sample: u16, neighbours: Option<[u16; 2]>, depth: u8) -> Result<u16> {
        if !(8..=12).contains(&depth) {
            return Err(invalid("unsupported SAO bit depth"));
        }
        let max = (1i32 << depth) - 1;
        if i32::from(sample) > max || neighbours.into_iter().flatten().any(|v| i32::from(v) > max) {
            return Err(invalid("SAO input sample exceeds bit depth"));
        }
        let limit = (1i16 << (depth - 5).min(5)) - 1;
        let offset = match self {
            Self::Off => 0,
            Self::Band { position, offsets } => {
                if position > 31 || offsets.iter().any(|v| !(-limit..=limit).contains(v)) {
                    return Err(invalid("invalid SAO band parameters"));
                }
                let band = ((sample >> (depth - 5)) as u8).wrapping_sub(position) & 31;
                offsets.get(usize::from(band)).copied().unwrap_or(0)
            }
            Self::Edge { class, offsets } => {
                if class > 3
                    || offsets[..2].iter().any(|v| !(0..=limit).contains(v))
                    || offsets[2..].iter().any(|v| !(-limit..=0).contains(v))
                {
                    return Err(invalid("invalid SAO edge parameters"));
                }
                if let Some([a, b]) = neighbours {
                    let sum = (i32::from(sample) - i32::from(a)).signum()
                        + (i32::from(sample) - i32::from(b)).signum();
                    match sum {
                        -2 => offsets[0],
                        -1 => offsets[1],
                        1 => offsets[2],
                        2 => offsets[3],
                        _ => 0,
                    }
                } else {
                    0
                }
            }
        };
        Ok((i32::from(sample) + i32::from(offset)).clamp(0, max) as u16)
    }
}

fn fixed(b: &mut impl ResidualBins, count: u8) -> Result<u8> {
    let mut value = 0;
    for _ in 0..count {
        value = (value << 1) | u8::from(b.bypass()?);
    }
    Ok(value)
}

/// Pass neighbour parameters only when merging is allowed by CTU position,
/// slice-segment and tile boundaries. Depths are Y/C. Main/Main10 offsets need
/// no range-extension scaling. Abort the slice if reading fails.
pub fn read_ctu(
    b: &mut impl ResidualBins,
    enabled: [bool; 2],
    depths: [u8; 2],
    left: Option<&CtuSao>,
    up: Option<&CtuSao>,
) -> Result<CtuSao> {
    if depths.iter().any(|d| !(8..=12).contains(d)) {
        return Err(invalid("unsupported HEVC SAO bit depth"));
    }
    if enabled == [false, false] {
        return Ok([Sao::Off; 3]);
    }
    if let Some(parameters) = left {
        if b.decision(Syntax::SaoMerge, 0)? {
            return Ok(*parameters);
        }
    }
    if let Some(parameters) = up {
        if b.decision(Syntax::SaoMerge, 0)? {
            return Ok(*parameters);
        }
    }
    let mut output = [Sao::Off; 3];
    let mut kind = 0;
    let mut edge_class = 0;
    for component in 0..3 {
        let depth_index = usize::from(component != 0);
        if !enabled[depth_index] {
            continue;
        }
        if component < 2 {
            kind = if b.decision(Syntax::SaoType, 0)? {
                1 + u8::from(b.bypass()?)
            } else {
                0
            };
        }
        if kind == 0 {
            continue;
        }
        let limit = (1i16 << (depths[depth_index] - 5).min(5)) - 1;
        let mut offsets = [0i16; 4];
        for value in &mut offsets {
            while *value < limit && b.bypass()? {
                *value += 1;
            }
        }
        output[component] = if kind == 1 {
            for value in &mut offsets {
                if *value != 0 && b.bypass()? {
                    *value = -*value;
                }
            }
            Sao::Band {
                position: fixed(b, 5)?,
                offsets,
            }
        } else {
            if component < 2 {
                edge_class = fixed(b, 2)?;
            }
            offsets[2] = -offsets[2];
            offsets[3] = -offsets[3];
            Sao::Edge {
                class: edge_class,
                offsets,
            }
        };
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_categories_missing_neighbours_and_directions() {
        for class in 0..4 {
            let mode = Sao::Edge {
                class,
                offsets: [7, 3, -2, -6],
            };
            for (pair, expected) in [
                ([101, 101], 107),
                ([100, 101], 103),
                ([99, 101], 100),
                ([100, 100], 100),
                ([99, 100], 98),
                ([99, 99], 94),
            ] {
                assert_eq!(mode.apply(100, Some(pair), 8).unwrap(), expected);
            }
            assert_eq!(mode.apply(100, None, 8).unwrap(), 100);
            let [a, b] = mode.neighbours().unwrap().unwrap();
            assert_eq!(a, [-b[0], -b[1]]);
        }
        assert!(
            Sao::Edge {
                class: 4,
                offsets: [0; 4]
            }
            .neighbours()
            .is_err()
        );
    }
    #[test]
    fn band_wrap_depth_scaling_clipping_and_invalid_parameters() {
        let mode = Sao::Band {
            position: 31,
            offsets: [7, -7, 2, -2],
        };
        for depth in [8, 10, 12] {
            let step = 1u16 << (depth - 5);
            assert_eq!(mode.apply(0, None, depth).unwrap(), 0);
            assert_eq!(mode.apply(step, None, depth).unwrap(), step + 2);
            assert_eq!(mode.apply(2 * step, None, depth).unwrap(), 2 * step - 2);
            assert_eq!(mode.apply(3 * step, None, depth).unwrap(), 3 * step);
            let max = (1 << depth) - 1;
            assert_eq!(mode.apply(max, None, depth).unwrap(), max);
        }
        assert!(mode.apply(256, None, 8).is_err());
        assert!(mode.apply(0, None, 7).is_err());
        assert!(
            Sao::Band {
                position: 32,
                offsets: [0; 4]
            }
            .apply(0, None, 8)
            .is_err()
        );
        assert!(
            Sao::Edge {
                class: 0,
                offsets: [-1, 0, 0, 0]
            }
            .apply(0, None, 8)
            .is_err()
        );
    }
    use std::collections::VecDeque;
    enum Bin {
        C(Syntax, bool),
        B(bool),
    }
    struct Script(VecDeque<Bin>);
    impl ResidualBins for Script {
        fn decision(&mut self, s: Syntax, inc: usize) -> Result<bool> {
            assert_eq!(inc, 0);
            match self.0.pop_front() {
                Some(Bin::C(expected, v)) => {
                    assert_eq!(
                        std::mem::discriminant(&s),
                        std::mem::discriminant(&expected)
                    );
                    Ok(v)
                }
                _ => Err(invalid("missing context bin")),
            }
        }
        fn bypass(&mut self) -> Result<bool> {
            match self.0.pop_front() {
                Some(Bin::B(v)) => Ok(v),
                _ => Err(invalid("missing bypass bin")),
            }
        }
    }
    #[test]
    fn merges_short_circuit_and_disabled_slices_consume_nothing() {
        let neighbour = [Sao::Band {
            position: 31,
            offsets: [1, -2, 3, 0],
        }; 3];
        let mut b = Script(VecDeque::from([Bin::C(Syntax::SaoMerge, true)]));
        assert_eq!(
            read_ctu(
                &mut b,
                [true, true],
                [8, 8],
                Some(&neighbour),
                Some(&neighbour)
            )
            .unwrap(),
            neighbour
        );
        assert!(b.0.is_empty());
        let mut b = Script(VecDeque::from([
            Bin::C(Syntax::SaoMerge, false),
            Bin::C(Syntax::SaoMerge, true),
        ]));
        assert_eq!(
            read_ctu(
                &mut b,
                [true, true],
                [8, 8],
                Some(&[Sao::Off; 3]),
                Some(&neighbour)
            )
            .unwrap(),
            neighbour
        );
        assert!(b.0.is_empty());
        assert_eq!(
            read_ctu(&mut b, [false, false], [8, 8], Some(&neighbour), None).unwrap(),
            [Sao::Off; 3]
        );
    }
    #[test]
    fn chroma_edge_type_and_class_are_shared_but_offsets_are_separate() {
        let mut b = Script(VecDeque::from([
            Bin::C(Syntax::SaoType, true),
            Bin::B(true),
        ]));
        // Cb absolute offsets 0,1,2,7. Maximum unary value has no final zero.
        for value in [0, 1, 2, 7] {
            for _ in 0..value {
                b.0.push_back(Bin::B(true));
            }
            if value < 7 {
                b.0.push_back(Bin::B(false));
            }
        }
        b.0.extend([Bin::B(true), Bin::B(false)]); // edge class 2
        for _ in 0..4 {
            b.0.push_back(Bin::B(false));
        } // Cr zero offsets; no class bins
        assert_eq!(
            read_ctu(&mut b, [false, true], [8, 8], None, None).unwrap(),
            [
                Sao::Off,
                Sao::Edge {
                    class: 2,
                    offsets: [0, 1, -2, -7]
                },
                Sao::Edge {
                    class: 2,
                    offsets: [0; 4]
                }
            ]
        );
        assert!(b.0.is_empty());
    }
    #[test]
    fn ten_bit_band_offsets_signs_and_position() {
        let mut b = Script(VecDeque::from([
            Bin::C(Syntax::SaoType, true),
            Bin::B(false),
        ]));
        for value in [31, 0, 1, 0] {
            for _ in 0..value {
                b.0.push_back(Bin::B(true));
            }
            if value < 31 {
                b.0.push_back(Bin::B(false));
            }
        }
        b.0.extend([Bin::B(true), Bin::B(false)]); // signs for nonzero offsets only
        for bit in [true, true, true, true, true] {
            b.0.push_back(Bin::B(bit));
        }
        assert_eq!(
            read_ctu(&mut b, [true, false], [10, 10], None, None).unwrap(),
            [
                Sao::Band {
                    position: 31,
                    offsets: [-31, 0, 1, 0]
                },
                Sao::Off,
                Sao::Off
            ]
        );
        assert!(b.0.is_empty());
        assert!(read_ctu(&mut b, [true, false], [10, 10], None, None).is_err());
        assert!(read_ctu(&mut b, [false, false], [7, 8], None, None).is_err());
    }
}
