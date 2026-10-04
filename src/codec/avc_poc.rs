//! H.264 picture order count types 0/1/2, section 8.2.1.
//! Invoke once per decoded picture, not once per slice. Gap-inferred pictures
//! must be supplied by the decoded-picture-buffer layer when applicable.
use super::{
    avc::{PictureOrder, Sps},
    avc_slice::{MemoryOperation, SliceHeader},
};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldOrder {
    pub top: Option<i32>,
    pub bottom: Option<i32>,
}
impl FieldOrder {
    pub fn picture(self) -> i32 {
        match (self.top, self.bottom) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => unreachable!("a picture has at least one field"),
        }
    }
    fn reset(self) -> Result<Self> {
        let origin = i64::from(self.picture());
        Ok(Self {
            top: self
                .top
                .map(|v| checked(i64::from(v) - origin))
                .transpose()?,
            bottom: self
                .bottom
                .map(|v| checked(i64::from(v) - origin))
                .transpose()?,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedPoc {
    /// Used while decoding this picture, including temporal prediction.
    pub before_marking: FieldOrder,
    /// MMCO 5 subtracts this picture's minimum POC after decoding.
    pub after_marking: FieldOrder,
    pub frame_num_offset: i32,
}
#[derive(Clone, Default)]
pub struct PocDecoder {
    config: Option<(u8, PictureOrder)>,
    previous_frame_num: u32,
    previous_frame_offset: i32,
    reference_msb: i32,
    reference_lsb: i32,
}
fn checked(value: i64) -> Result<i32> {
    i32::try_from(value).map_err(|_| invalid("picture order count exceeds signed 32-bit range"))
}
impl PocDecoder {
    pub fn new() -> Self {
        Self::default()
    }
    /// Commits only after all calculations and invariants succeed. On a decoding
    /// failure after this call, discard/reset the picture pipeline before reuse.
    pub fn decode(&mut self, sps: &Sps, header: &SliceHeader) -> Result<DecodedPoc> {
        if !(4..=16).contains(&sps.frame_num_bits)
            || header.frame_num >= (1u32 << sps.frame_num_bits)
            || (sps.frame_mbs_only && header.field_pic)
            || (header.idr && (header.frame_num != 0 || header.nal_ref_idc == 0))
        {
            return Err(invalid("invalid POC picture parameters"));
        }
        if !header.idr {
            match &self.config {
                None => return Err(invalid("POC sequence must start at an IDR picture")),
                Some((bits, order))
                    if *bits != sps.frame_num_bits || order != &sps.picture_order =>
                {
                    return Err(invalid("POC configuration changed without IDR"));
                }
                _ => {}
            }
        }
        let reset = header
            .memory_operations
            .iter()
            .any(|op| matches!(op, MemoryOperation::Reset));
        if reset && header.nal_ref_idc == 0 {
            return Err(invalid("MMCO reset in a non-reference picture"));
        }
        let offset = if header.idr {
            0
        } else {
            checked(
                i64::from(self.previous_frame_offset)
                    + if self.previous_frame_num > header.frame_num {
                        1i64 << sps.frame_num_bits
                    } else {
                        0
                    },
            )?
        };
        let mut next_msb = self.reference_msb;
        let mut next_lsb = self.reference_lsb;
        let (top, bottom) = match &sps.picture_order {
            PictureOrder::Lsb { bits } => {
                if !(4..=16).contains(bits) {
                    return Err(invalid("invalid POC LSB width"));
                }
                let max = 1i64 << bits;
                let lsb = i64::from(
                    header
                        .poc_lsb
                        .ok_or_else(|| invalid("missing pic_order_cnt_lsb"))?,
                );
                if lsb >= max {
                    return Err(invalid("pic_order_cnt_lsb exceeds width"));
                }
                let (prev_msb, prev_lsb) = if header.idr {
                    (0, 0)
                } else {
                    (i64::from(self.reference_msb), i64::from(self.reference_lsb))
                };
                let msb = if lsb < prev_lsb && prev_lsb - lsb >= max / 2 {
                    prev_msb + max
                } else if lsb > prev_lsb && lsb - prev_lsb > max / 2 {
                    prev_msb - max
                } else {
                    prev_msb
                };
                let msb = checked(msb)?;
                let top = i64::from(msb) + lsb;
                if header.nal_ref_idc != 0 {
                    next_msb = msb;
                    next_lsb = lsb as i32;
                }
                (
                    top,
                    top + if header.field_pic {
                        0
                    } else {
                        i64::from(header.delta_poc_bottom)
                    },
                )
            }
            PictureOrder::Cycle {
                always_zero,
                non_ref_offset,
                top_bottom_offset,
                offsets,
            } => {
                if offsets.len() > 255 || (*always_zero && header.delta_poc != [0; 2]) {
                    return Err(invalid("invalid POC cycle parameters"));
                }
                let mut absolute = if offsets.is_empty() {
                    0
                } else {
                    i64::from(offset) + i64::from(header.frame_num)
                };
                if header.nal_ref_idc == 0 && absolute > 0 {
                    absolute -= 1;
                }
                let mut expected = 0i64;
                if absolute > 0 {
                    let size = offsets.len() as i64;
                    let cycle = (absolute - 1) / size;
                    let inside = ((absolute - 1) % size) as usize;
                    let sum: i64 = offsets.iter().map(|&v| i64::from(v)).sum();
                    expected = cycle * sum
                        + offsets[..=inside]
                            .iter()
                            .map(|&v| i64::from(v))
                            .sum::<i64>();
                }
                if header.nal_ref_idc == 0 {
                    expected += i64::from(*non_ref_offset);
                }
                let top = expected + i64::from(header.delta_poc[0]);
                let bottom = top
                    + i64::from(*top_bottom_offset)
                    + if header.field_pic {
                        0
                    } else {
                        i64::from(header.delta_poc[1])
                    };
                (top, bottom)
            }
            PictureOrder::DecodeOrder => {
                let order = if header.idr {
                    0
                } else {
                    2 * (i64::from(offset) + i64::from(header.frame_num))
                        - i64::from(header.nal_ref_idc == 0)
                };
                (order, order)
            }
        };
        let before = FieldOrder {
            top: if header.field_pic && header.bottom_field {
                None
            } else {
                Some(checked(top)?)
            },
            bottom: if header.field_pic && !header.bottom_field {
                None
            } else {
                Some(checked(bottom)?)
            },
        };
        if header.idr && before.picture() != 0 {
            return Err(invalid("IDR picture order count is not zero"));
        }
        let after = if reset { before.reset()? } else { before };
        if reset {
            next_msb = 0;
            next_lsb = after.top.unwrap_or(0);
        }
        if header.idr {
            self.config = Some((sps.frame_num_bits, sps.picture_order.clone()));
        }
        self.previous_frame_num = if reset { 0 } else { header.frame_num };
        self.previous_frame_offset = if reset { 0 } else { offset };
        self.reference_msb = next_msb;
        self.reference_lsb = next_lsb;
        Ok(DecodedPoc {
            before_marking: before,
            after_marking: after,
            frame_num_offset: offset,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::avc::Pps;
    use super::*;
    fn fixture() -> (Sps, SliceHeader) {
        let h = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        let bytes: Vec<_> = (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
            .collect();
        let sps = Sps::parse(&bytes).unwrap();
        let pps = Pps::parse(&[0x68, 0xce, 0x09, 0xc8], &sps).unwrap();
        let header = SliceHeader::parse(&[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80], &sps, &pps).unwrap();
        (sps, header)
    }
    #[test]
    fn type_zero_reordering_wrap_and_non_reference_history() {
        let (mut s, mut h) = fixture();
        s.picture_order = PictureOrder::Lsb { bits: 4 };
        h.poc_lsb = Some(0);
        let mut decoder = PocDecoder::new();
        assert_eq!(decoder.decode(&s, &h).unwrap().before_marking.picture(), 0);
        h.idr = false;
        for (lsb, reference, expected) in [
            (6, 1, 6),
            (2, 0, 2),
            (4, 0, 4),
            (12, 1, 12),
            (8, 0, 8),
            (10, 0, 10),
            (2, 1, 18),
            (14, 0, 14),
            (0, 0, 16),
        ] {
            h.poc_lsb = Some(lsb);
            h.nal_ref_idc = reference;
            assert_eq!(
                decoder.decode(&s, &h).unwrap().before_marking.picture(),
                expected
            );
        }
    }
    #[test]
    fn type_one_cycles_and_type_two_frame_num_wrap() {
        let (mut s, mut h) = fixture();
        s.frame_num_bits = 4;
        s.picture_order = PictureOrder::Cycle {
            always_zero: false,
            non_ref_offset: -1,
            top_bottom_offset: 1,
            offsets: vec![2, 3],
        };
        let mut decoder = PocDecoder::new();
        assert_eq!(
            decoder.decode(&s, &h).unwrap().before_marking,
            FieldOrder {
                top: Some(0),
                bottom: Some(1)
            }
        );
        h.idr = false;
        h.frame_num = 1;
        assert_eq!(decoder.decode(&s, &h).unwrap().before_marking.picture(), 2);
        h.frame_num = 2;
        h.nal_ref_idc = 0;
        assert_eq!(decoder.decode(&s, &h).unwrap().before_marking.picture(), 1);
        h.nal_ref_idc = 1;
        assert_eq!(decoder.decode(&s, &h).unwrap().before_marking.picture(), 5);
        h.idr = true;
        h.frame_num = 0;
        s.picture_order = PictureOrder::DecodeOrder;
        decoder.decode(&s, &h).unwrap();
        h.idr = false;
        for (frame, reference, expected) in [(15, 1, 30), (0, 1, 32), (1, 0, 33), (1, 1, 34)] {
            h.frame_num = frame;
            h.nal_ref_idc = reference;
            assert_eq!(
                decoder.decode(&s, &h).unwrap().before_marking.picture(),
                expected
            );
        }
    }
    #[test]
    fn mmco_five_reset_field_pictures_and_error_does_not_commit() {
        let (mut s, mut h) = fixture();
        s.picture_order = PictureOrder::Lsb { bits: 4 };
        h.poc_lsb = Some(0);
        let mut decoder = PocDecoder::new();
        decoder.decode(&s, &h).unwrap();
        h.idr = false;
        h.poc_lsb = Some(6);
        h.delta_poc_bottom = -2;
        h.memory_operations = vec![MemoryOperation::Reset];
        let result = decoder.decode(&s, &h).unwrap();
        assert_eq!(
            result.before_marking,
            FieldOrder {
                top: Some(6),
                bottom: Some(4)
            }
        );
        assert_eq!(
            result.after_marking,
            FieldOrder {
                top: Some(2),
                bottom: Some(0)
            }
        );
        h.memory_operations.clear();
        h.delta_poc_bottom = 0;
        h.poc_lsb = Some(10);
        assert_eq!(decoder.decode(&s, &h).unwrap().before_marking.picture(), 10); // exact half-range forward does not wrap
        h.poc_lsb = Some(100);
        assert!(decoder.decode(&s, &h).is_err());
        h.poc_lsb = Some(2);
        assert_eq!(decoder.decode(&s, &h).unwrap().before_marking.picture(), 18); // exact half-range backward wraps
        s.frame_mbs_only = false;
        h.field_pic = true;
        h.bottom_field = true;
        h.poc_lsb = Some(4);
        assert_eq!(
            decoder.decode(&s, &h).unwrap().before_marking,
            FieldOrder {
                top: None,
                bottom: Some(20)
            }
        );
    }
}
