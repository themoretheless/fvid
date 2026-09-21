//! CABAC inter syntax binarizations (H.264 9.3.2 and tables 9-37..9-41).
//! Neighbour conditions are supplied by the slice's spatial context grids.
use super::{avc_cabac::AvcCabac, avc_slice::SliceType};
use crate::{Result, invalid};

pub trait InterBins {
    fn decision(&mut self, context: usize) -> Result<bool>;
    fn bypass(&mut self) -> Result<bool>;
    fn terminate(&mut self) -> Result<bool>;
}
impl InterBins for AvcCabac<'_> {
    fn decision(&mut self, context: usize) -> Result<bool> {
        AvcCabac::decision(self, context)
    }
    fn bypass(&mut self) -> Result<bool> {
        AvcCabac::bypass(self)
    }
    fn terminate(&mut self) -> Result<bool> {
        AvcCabac::terminate(self)
    }
}
fn base(slice: SliceType, p: usize, b: usize) -> Result<usize> {
    match slice {
        SliceType::P => Ok(p),
        SliceType::B => Ok(b),
        _ => Err(invalid("CABAC inter syntax needs P/B slice")),
    }
}
/// Neighbours contribute only when available and not skipped.
pub fn skip(bins: &mut impl InterBins, slice: SliceType, non_skip: [bool; 2]) -> Result<bool> {
    bins.decision(base(slice, 11, 24)? + usize::from(non_skip[0]) + usize::from(non_skip[1]))
}
fn intra_suffix(bins: &mut impl InterBins, offset: usize) -> Result<u8> {
    if !bins.decision(offset)? {
        return Ok(0);
    }
    if bins.terminate()? {
        return Ok(25);
    }
    let luma = u8::from(bins.decision(offset + 1)?);
    let chroma = if bins.decision(offset + 2)? {
        1 + u8::from(bins.decision(offset + 2)?)
    } else {
        0
    };
    let mode = 2 * u8::from(bins.decision(offset + 3)?) + u8::from(bins.decision(offset + 3)?);
    Ok(1 + mode + 4 * chroma + 12 * luma)
}
/// Returns the standard numeric mb_type, including embedded intra types.
/// B neighbours contribute when available and neither skip nor direct.
pub fn macroblock_type(
    bins: &mut impl InterBins,
    slice: SliceType,
    non_direct: [bool; 2],
) -> Result<u8> {
    match slice {
        SliceType::P => {
            if bins.decision(14)? {
                return Ok(5 + intra_suffix(bins, 17)?);
            }
            let second = bins.decision(15)?;
            let third = bins.decision(if second { 17 } else { 16 })?;
            Ok(match (second, third) {
                (false, false) => 0,
                (true, true) => 1,
                (true, false) => 2,
                (false, true) => 3,
            })
        }
        SliceType::B => {
            let increment = usize::from(non_direct[0]) + usize::from(non_direct[1]);
            let prefix = decode_tree(bins, &B_MB, |index, bits| match index {
                0 => 27 + increment,
                1 => 30,
                2 => {
                    if bits & 1 != 0 {
                        31
                    } else {
                        32
                    }
                }
                _ => 32,
            })?;
            if prefix == 23 {
                Ok(23 + intra_suffix(bins, 32)?)
            } else {
                Ok(prefix)
            }
        }
        _ => Err(invalid("CABAC inter syntax needs P/B slice")),
    }
}
const B_MB: [&str; 24] = [
    "0", "100", "101", "110000", "110001", "110010", "110011", "110100", "110101", "110110",
    "110111", "111110", "1110000", "1110001", "1110010", "1110011", "1110100", "1110101",
    "1110110", "1110111", "1111000", "1111001", "111111", "111101",
];
const P_SUB: [&str; 4] = ["1", "00", "011", "010"];
const B_SUB: [&str; 13] = [
    "0", "100", "101", "11000", "11001", "11010", "11011", "111000", "111001", "111010", "111011",
    "11110", "11111",
];
fn decode_tree(
    bins: &mut impl InterBins,
    codes: &[&str],
    context: impl Fn(usize, u8) -> usize,
) -> Result<u8> {
    let mut bits = 0u8;
    for index in 0..7 {
        bits = (bits << 1) | u8::from(bins.decision(context(index, bits))?);
        for (value, code) in codes.iter().enumerate() {
            if code.len() == index + 1
                && code.bytes().fold(0u8, |n, b| (n << 1) | (b - b'0')) == bits
            {
                return Ok(value as u8);
            }
        }
    }
    Err(invalid("invalid CABAC macroblock binarization"))
}
pub fn sub_macroblock_type(bins: &mut impl InterBins, slice: SliceType) -> Result<u8> {
    match slice {
        SliceType::P => decode_tree(bins, &P_SUB, |index, _| 21 + index),
        SliceType::B => decode_tree(bins, &B_SUB, |index, bits| match index {
            0 => 36,
            1 => 37,
            2 => {
                if bits & 1 != 0 {
                    38
                } else {
                    39
                }
            }
            _ => 39,
        }),
        _ => Err(invalid("CABAC inter syntax needs P/B slice")),
    }
}
/// Conditions are true for available, non-direct neighbours using this list
/// with a positive reference index (progressive-frame derivation).
pub fn reference_index(bins: &mut impl InterBins, positive: [bool; 2], active: u32) -> Result<u8> {
    if active == 0 || active > 32 {
        return Err(invalid("invalid CABAC active reference count"));
    }
    if active == 1 {
        return Ok(0);
    }
    let mut value = 0usize;
    loop {
        let ctx = match value {
            0 => 54 + usize::from(positive[0]) + 2 * usize::from(positive[1]),
            1 => 58,
            _ => 59,
        };
        if !bins.decision(ctx)? {
            return Ok(value as u8);
        }
        value += 1;
        if value >= active as usize {
            return Err(invalid("CABAC reference index exceeds active list"));
        }
    }
}
/// Neighbour magnitudes must already reflect list availability (zero when
/// unavailable, intra, skip, direct or using only the other list).
pub fn motion_difference(
    bins: &mut impl InterBins,
    component: usize,
    magnitudes: [u32; 2],
) -> Result<i32> {
    if component > 1 {
        return Err(invalid("invalid CABAC motion component"));
    }
    let sum = u64::from(magnitudes[0]) + u64::from(magnitudes[1]);
    let increment = if sum < 3 {
        0
    } else if sum > 32 {
        2
    } else {
        1
    };
    let offset = if component == 0 { 40 } else { 47 };
    let mut value = 0u32;
    while value < 9 {
        let ctx = offset
            + if value == 0 {
                increment
            } else {
                (value as usize + 2).min(6)
            };
        if !bins.decision(ctx)? {
            break;
        }
        value += 1;
    }
    if value == 9 {
        let mut k = 3;
        while bins.bypass()? {
            if k >= 30 {
                return Err(invalid("CABAC motion difference overflow"));
            }
            value = value
                .checked_add(1u32 << k)
                .ok_or_else(|| invalid("CABAC motion difference overflow"))?;
            k += 1;
        }
        for bit in (0..k).rev() {
            if bins.bypass()? {
                value = value
                    .checked_add(1u32 << bit)
                    .ok_or_else(|| invalid("CABAC motion difference overflow"))?;
            }
        }
    }
    let value = i32::try_from(value).map_err(|_| invalid("CABAC motion difference overflow"))?;
    if value != 0 && bins.bypass()? {
        Ok(-value)
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Script(VecDeque<(i32, bool)>);
    impl Script {
        fn new(values: &[(i32, bool)]) -> Self {
            Self(values.iter().copied().collect())
        }
        fn read(&mut self, expected: i32) -> Result<bool> {
            let (context, value) = self.0.pop_front().expect("unexpected bin read");
            assert_eq!(context, expected);
            Ok(value)
        }
        fn done(self) {
            assert!(self.0.is_empty());
        }
    }
    impl InterBins for Script {
        fn decision(&mut self, c: usize) -> Result<bool> {
            self.read(c as i32)
        }
        fn bypass(&mut self) -> Result<bool> {
            self.read(-1)
        }
        fn terminate(&mut self) -> Result<bool> {
            self.read(-2)
        }
    }
    #[test]
    fn real_cabac_p_skip_picture_reaches_exact_slice_termination() {
        fn hex(s: &str) -> Vec<u8> {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect()
        }
        let sps = super::super::avc::Sps::parse(&hex("674d400ad9096c0440000003004000000c83c48992"))
            .unwrap();
        let pps = super::super::avc::Pps::parse(&hex("68eb83cb20"), &sps).unwrap();
        let header =
            super::super::avc_slice::SliceHeader::parse(&hex("419a390afffe56"), &sps, &pps)
                .unwrap();
        assert_eq!((sps.width_mbs, sps.height_map_units), (2, 2));
        let mut bins = AvcCabac::new(
            &header.rbsp,
            header.entropy_bit_offset,
            header.slice_type,
            header.cabac_init_idc as u8,
            header.slice_qp,
        )
        .unwrap();
        for address in 0..4 {
            assert!(skip(&mut bins, SliceType::P, [false; 2]).unwrap());
            assert_eq!(bins.terminate().unwrap(), address == 3);
        }
        bins.finish_slice().unwrap();
    }
    #[test]
    fn p_types_and_embedded_intra_select_normative_contexts() {
        for (tail, result) in [(false, 0), (true, 3)] {
            let mut b = Script::new(&[(14, false), (15, false), (16, tail)]);
            assert_eq!(
                macroblock_type(&mut b, SliceType::P, [false; 2]).unwrap(),
                result
            );
            b.done();
        }
        for (tail, result) in [(false, 2), (true, 1)] {
            let mut b = Script::new(&[(14, false), (15, true), (17, tail)]);
            assert_eq!(
                macroblock_type(&mut b, SliceType::P, [false; 2]).unwrap(),
                result
            );
            b.done();
        }
        let mut b = Script::new(&[
            (14, true),
            (17, true),
            (-2, false),
            (18, true),
            (19, true),
            (19, true),
            (20, true),
            (20, true),
        ]);
        assert_eq!(
            macroblock_type(&mut b, SliceType::P, [false; 2]).unwrap(),
            29
        );
        b.done();
        let mut b = Script::new(&[(14, true), (17, true), (-2, true)]);
        assert_eq!(
            macroblock_type(&mut b, SliceType::P, [false; 2]).unwrap(),
            30
        );
        b.done();
    }
    #[test]
    fn b_prefixes_and_subpartitions_follow_distinct_contexts() {
        let mut b = Script::new(&[(29, true), (30, false), (32, true)]);
        assert_eq!(macroblock_type(&mut b, SliceType::B, [true; 2]).unwrap(), 2);
        b.done();
        let mut b = Script::new(&[
            (27, true),
            (30, true),
            (31, true),
            (32, true),
            (32, false),
            (32, true),
            (32, false),
        ]);
        assert_eq!(
            macroblock_type(&mut b, SliceType::B, [false; 2]).unwrap(),
            23
        );
        b.done();
        let mut b = Script::new(&[(21, false), (22, true), (23, false)]);
        assert_eq!(sub_macroblock_type(&mut b, SliceType::P).unwrap(), 3);
        b.done();
        let mut b = Script::new(&[
            (36, true),
            (37, true),
            (38, true),
            (39, false),
            (39, true),
            (39, true),
        ]);
        assert_eq!(sub_macroblock_type(&mut b, SliceType::B).unwrap(), 10);
        b.done();
    }
    #[test]
    fn reference_unary_and_motion_escape_boundaries() {
        let mut b = Script::new(&[(57, true), (58, true), (59, false)]);
        assert_eq!(reference_index(&mut b, [true; 2], 3).unwrap(), 2);
        b.done();
        let mut b = Script::new(&[(54, true), (58, true)]);
        assert!(reference_index(&mut b, [false; 2], 2).is_err());
        b.done();
        let mut b = Script::new(&[(49, false)]);
        assert_eq!(motion_difference(&mut b, 1, [16, 17]).unwrap(), 0);
        b.done();
        let mut values = vec![
            (41, true),
            (43, true),
            (44, true),
            (45, true),
            (46, true),
            (46, true),
            (46, true),
            (46, true),
            (46, true),
        ];
        // abs=17: TU prefix=9, UEG3 suffix=1 0 0000, sign=1.
        values.extend([
            (-1, true),
            (-1, false),
            (-1, false),
            (-1, false),
            (-1, false),
            (-1, false),
            (-1, true),
        ]);
        let mut b = Script::new(&values);
        assert_eq!(motion_difference(&mut b, 0, [1, 2]).unwrap(), -17);
        b.done();
    }
}
