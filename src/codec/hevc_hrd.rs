//! H.265 Annex E.2.2/E.2.3 hypothetical reference decoder syntax.
use super::bits::BitReader;
use crate::{Result, invalid};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubPicture {
    pub tick_divisor: u16,
    pub removal_increment_bits: u8,
    pub in_picture_timing_sei: bool,
    pub output_delay_bits: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Common {
    pub nal_present: bool,
    pub vcl_present: bool,
    pub sub_picture: Option<SubPicture>,
    pub bit_rate_scale: u8,
    pub cpb_size_scale: u8,
    pub cpb_size_du_scale: u8,
    pub initial_removal_delay_bits: u8,
    pub removal_delay_bits: u8,
    pub output_delay_bits: u8,
}
impl Common {
    fn read(b: &mut BitReader<'_>) -> Result<Self> {
        let mut c = Self {
            nal_present: b.bit()?,
            vcl_present: b.bit()?,
            sub_picture: None,
            bit_rate_scale: 0,
            cpb_size_scale: 0,
            cpb_size_du_scale: 0,
            initial_removal_delay_bits: 24,
            removal_delay_bits: 24,
            output_delay_bits: 24,
        };
        if c.nal_present || c.vcl_present {
            if b.bit()? {
                c.sub_picture = Some(SubPicture {
                    tick_divisor: b.read(8)? as u16 + 2,
                    removal_increment_bits: b.read(5)? as u8 + 1,
                    in_picture_timing_sei: b.bit()?,
                    output_delay_bits: b.read(5)? as u8 + 1,
                });
            }
            c.bit_rate_scale = b.read(4)? as u8;
            c.cpb_size_scale = b.read(4)? as u8;
            if c.sub_picture.is_some() {
                c.cpb_size_du_scale = b.read(4)? as u8;
            }
            c.initial_removal_delay_bits = b.read(5)? as u8 + 1;
            c.removal_delay_bits = b.read(5)? as u8 + 1;
            c.output_delay_bits = b.read(5)? as u8 + 1;
        }
        Ok(c)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cpb {
    pub bit_rate_value: u32,
    pub size_value: u32,
    /// (CPB size, bit rate) in decoding-unit scale units.
    pub decoding_unit: Option<(u32, u32)>,
    pub constant_rate: bool,
}
fn value(b: &mut BitReader<'_>) -> Result<u32> {
    b.unsigned_golomb()?
        .checked_add(1)
        .ok_or_else(|| invalid("HEVC HRD value exceeds range"))
}
fn cpb(b: &mut BitReader<'_>, count: usize, sub_picture: bool) -> Result<Vec<Cpb>> {
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(Cpb {
            bit_rate_value: value(b)?,
            size_value: value(b)?,
            decoding_unit: if sub_picture {
                Some((value(b)?, value(b)?))
            } else {
                None
            },
            constant_rate: b.bit()?,
        });
    }
    Ok(entries)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubLayer {
    pub fixed_rate_general: bool,
    pub elemental_duration: Option<u16>,
    pub low_delay: bool,
    pub nal: Vec<Cpb>,
    pub vcl: Vec<Cpb>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hrd {
    pub common: Common,
    pub layers: Vec<SubLayer>,
}
impl Hrd {
    /// `inherited` supplies the preceding VPS HRD common syntax when
    /// cprms_present_flag is false; None reads common parameters from the stream.
    pub fn read(
        b: &mut BitReader<'_>,
        max_sub_layers_minus1: u8,
        inherited: Option<&Common>,
    ) -> Result<Self> {
        if max_sub_layers_minus1 > 6 {
            return Err(invalid("invalid HEVC HRD sublayer count"));
        }
        let common = match inherited {
            Some(c) => c.clone(),
            None => Common::read(b)?,
        };
        let mut layers = Vec::with_capacity(max_sub_layers_minus1 as usize + 1);
        for _ in 0..=max_sub_layers_minus1 {
            let fixed_rate_general = b.bit()?;
            let fixed = fixed_rate_general || b.bit()?;
            let elemental_duration = if fixed {
                let duration = b.unsigned_golomb()?;
                if duration > 2047 {
                    return Err(invalid("HEVC HRD elemental duration exceeds range"));
                }
                Some(duration as u16 + 1)
            } else {
                None
            };
            let low_delay = !fixed && b.bit()?;
            let count = if low_delay {
                1
            } else {
                let count = b.unsigned_golomb()?;
                if count > 31 {
                    return Err(invalid("HEVC HRD CPB count exceeds 32"));
                }
                count as usize + 1
            };
            let nal = if common.nal_present {
                cpb(b, count, common.sub_picture.is_some())?
            } else {
                Vec::new()
            };
            let vcl = if common.vcl_present {
                cpb(b, count, common.sub_picture.is_some())?
            } else {
                Vec::new()
            };
            layers.push(SubLayer {
                fixed_rate_general,
                elemental_duration,
                low_delay,
                nal,
                vcl,
            });
        }
        Ok(Self { common, layers })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nal_vcl_subpicture_low_delay_and_truncation() {
        let data = [224, 0, 128, 0, 0, 0, 26, 100, 148, 199, 16, 128];
        let mut bits = BitReader::new(&data);
        let h = Hrd::read(&mut bits, 0, None).unwrap();
        assert_eq!(h.common.sub_picture.as_ref().unwrap().tick_divisor, 2);
        assert!(h.common.sub_picture.as_ref().unwrap().in_picture_timing_sei);
        assert_eq!(h.common.removal_delay_bits, 1);
        assert!(h.layers[0].low_delay);
        assert_eq!(
            h.layers[0].nal,
            vec![Cpb {
                bit_rate_value: 1,
                size_value: 2,
                decoding_unit: Some((3, 4)),
                constant_rate: true
            }]
        );
        assert_eq!(
            h.layers[0].vcl,
            vec![Cpb {
                bit_rate_value: 5,
                size_value: 6,
                decoding_unit: Some((7, 8)),
                constant_rate: false
            }]
        );
        bits.finish_rbsp().unwrap();
        for end in 0..data.len() - 1 {
            assert!(Hrd::read(&mut BitReader::new(&data[..end]), 0, None).is_err());
        }
    }
    #[test]
    fn absent_hrd_and_inherited_common_do_not_consume_extra_fields() {
        // no NAL/VCL HRD; fixed rate, duration=1, CPB count=1, stop.
        let mut b = BitReader::new(&[0x3c]);
        let h = Hrd::read(&mut b, 0, None).unwrap();
        assert_eq!(h.layers[0].elemental_duration, Some(1));
        assert!(h.layers[0].nal.is_empty());
        b.finish_rbsp().unwrap();
        // common omitted, variable rate, low delay; stop.
        let mut b = BitReader::new(&[0x30]);
        let inherited = Hrd::read(&mut b, 0, Some(&h.common)).unwrap();
        assert!(inherited.layers[0].low_delay);
        assert_eq!(inherited.layers[0].elemental_duration, None);
        b.finish_rbsp().unwrap();
        assert!(Hrd::read(&mut BitReader::new(&[]), 7, None).is_err());
    }
}
