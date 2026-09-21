//! H.265 base-layer VPS syntax and shared temporal-sublayer DPB limits.
use super::{bits::BitReader, hevc_nal::NalRbsp, hevc_profile::ProfileTierLevel};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ordering {
    pub max_decoded_pictures: u8,
    pub max_reorder_pictures: u8,
    pub max_latency_increase_plus1: u32,
}
/// Shared by VPS and SPS. Level/geometry-dependent DPB limits need a later SPS
/// conformance check; this reader enforces the absolute 16-picture bound.
pub fn read_ordering(bits: &mut BitReader<'_>, max_sub_layers_minus1: u8) -> Result<Vec<Ordering>> {
    if max_sub_layers_minus1 > 6 {
        return Err(invalid("invalid HEVC ordering sublayer count"));
    }
    let all = bits.bit()?;
    let count = usize::from(max_sub_layers_minus1) + 1;
    let mut output = vec![Ordering::default(); count];
    for i in if all { 0 } else { count - 1 }..count {
        let dpb_minus1 = bits.unsigned_golomb()?;
        let reorder = bits.unsigned_golomb()?;
        let latency = bits.unsigned_golomb()?;
        if dpb_minus1 > 15 || reorder > dpb_minus1 || latency == u32::MAX {
            return Err(invalid("invalid HEVC decoded-picture ordering limits"));
        }
        output[i] = Ordering {
            max_decoded_pictures: (dpb_minus1 + 1) as u8,
            max_reorder_pictures: reorder as u8,
            max_latency_increase_plus1: latency,
        };
        if all
            && i > 0
            && (output[i].max_decoded_pictures < output[i - 1].max_decoded_pictures
                || output[i].max_reorder_pictures < output[i - 1].max_reorder_pictures)
        {
            return Err(invalid("HEVC sublayer ordering limits decrease"));
        }
    }
    if !all {
        let highest = output[count - 1];
        output.fill(highest);
    }
    Ok(output)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub units_in_tick: u32,
    pub time_scale: u32,
    pub ticks_per_poc: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vps {
    pub id: u8,
    pub temporal_id_nesting: bool,
    pub profile: ProfileTierLevel,
    pub ordering: Vec<Ordering>,
    pub timing: Option<Timing>,
    pub hrd: Option<super::hevc_hrd::Hrd>,
}
impl Vps {
    pub fn parse(nal: &[u8], budget: usize) -> Result<Self> {
        let rbsp = NalRbsp::parse(nal, budget)?;
        rbsp.header.require_base_layer()?;
        if rbsp.header.unit_type != 32 || rbsp.header.temporal_id != 0 {
            return Err(invalid("expected base-layer HEVC VPS at temporal ID zero"));
        }
        let mut bits = BitReader::new(&rbsp.bytes);
        let id = bits.read(4)? as u8;
        let internal = bits.bit()?;
        let available = bits.bit()?;
        let max_layers_minus1 = bits.read(6)?;
        let max_sub_layers_minus1 = bits.read(3)? as u8;
        let temporal_id_nesting = bits.bit()?;
        if !internal || !available || max_layers_minus1 != 0 {
            return Err(invalid(
                "multilayer or external-base HEVC VPS is not implemented",
            ));
        }
        if max_sub_layers_minus1 == 0 && !temporal_id_nesting {
            return Err(invalid("single-sublayer VPS requires temporal nesting"));
        }
        // H.265 7.4.3.1 explicitly requires decoders to ignore this reserved value.
        bits.skip(16)?;
        let profile = ProfileTierLevel::read(&mut bits, true, max_sub_layers_minus1)?;
        let ordering = read_ordering(&mut bits, max_sub_layers_minus1)?;
        if bits.read(6)? != 0 || bits.unsigned_golomb()? != 0 {
            return Err(invalid("multiple HEVC layer sets are not implemented"));
        }
        let mut hrd = None;
        let timing = if bits.bit()? {
            let units_in_tick = bits.read(32)?;
            let time_scale = bits.read(32)?;
            if units_in_tick == 0 || time_scale == 0 {
                return Err(invalid("zero HEVC VPS clock"));
            }
            let ticks_per_poc = if bits.bit()? {
                Some(u64::from(
                    bits.unsigned_golomb()?
                        .checked_add(1)
                        .ok_or_else(|| invalid("VPS POC tick count exceeds range"))?,
                ))
            } else {
                None
            };
            match bits.unsigned_golomb()? {
                0 => {}
                1 => {
                    if bits.unsigned_golomb()? != 0 {
                        return Err(invalid("invalid VPS HRD layer set index"));
                    }
                    hrd = Some(super::hevc_hrd::Hrd::read(
                        &mut bits,
                        max_sub_layers_minus1,
                        None,
                    )?);
                }
                _ => return Err(invalid("VPS HRD count exceeds layer sets")),
            }
            Some(Timing {
                units_in_tick,
                time_scale,
                ticks_per_poc,
            })
        } else {
            None
        };
        if bits.bit()? {
            return Err(invalid("HEVC VPS extension is not implemented"));
        }
        bits.finish_rbsp()?;
        Ok(Self {
            id,
            temporal_id_nesting,
            profile,
            ordering,
            timing,
            hrd,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ordering_bits(all: bool, values: &[[u32; 3]]) -> Vec<u8> {
        let mut bits = vec![all];
        for value in values.iter().flatten() {
            let code = u64::from(*value) + 1;
            let width = 64 - code.leading_zeros();
            bits.extend(std::iter::repeat_n(false, (width - 1) as usize));
            for shift in (0..width).rev() {
                bits.push(code & (1 << shift) != 0);
            }
        }
        bits.push(true);
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        bits.chunks(8)
            .map(|chunk| {
                chunk
                    .iter()
                    .fold(0, |byte, &bit| (byte << 1) | u8::from(bit))
            })
            .collect()
    }
    #[test]
    fn ordering_inference_monotonicity_and_bounds() {
        let data = ordering_bits(false, &[[4, 2, 5]]);
        let mut bits = BitReader::new(&data);
        let limits = read_ordering(&mut bits, 2).unwrap();
        assert_eq!(
            limits,
            vec![
                Ordering {
                    max_decoded_pictures: 5,
                    max_reorder_pictures: 2,
                    max_latency_increase_plus1: 5
                };
                3
            ]
        );
        bits.finish_rbsp().unwrap();
        let valid = ordering_bits(true, &[[1, 0, 0], [4, 2, 5]]);
        assert_eq!(
            read_ordering(&mut BitReader::new(&valid), 1).unwrap()[0].max_decoded_pictures,
            2
        );
        for invalid_values in [
            vec![[16, 0, 0]],
            vec![[1, 2, 0]],
            vec![[1, 0, u32::MAX]],
            vec![[4, 2, 0], [3, 2, 0]],
            vec![[4, 2, 0], [4, 1, 0]],
        ] {
            let data = ordering_bits(true, &invalid_values);
            assert!(
                read_ordering(&mut BitReader::new(&data), (invalid_values.len() - 1) as u8)
                    .is_err()
            );
        }
    }
    #[test]
    fn real_vps_and_every_truncation() {
        let hex = "40010c01ffff01600000030090000003000003001e959809";
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let vps = Vps::parse(&bytes, 1024).unwrap();
        assert_eq!(vps.id, 0);
        assert_eq!(vps.profile.profile.unwrap().idc, 1);
        assert_eq!(
            vps.ordering,
            vec![Ordering {
                max_decoded_pictures: 5,
                max_reorder_pictures: 2,
                max_latency_increase_plus1: 5
            }]
        );
        assert_eq!(vps.timing, None);
        for end in 0..bytes.len() {
            assert!(Vps::parse(&bytes[..end], 1024).is_err());
        }
        assert!(Vps::parse(&bytes, 1).is_err());
        let mut extra = bytes.clone();
        extra.push(0x80);
        assert!(Vps::parse(&extra, 1024).is_err());
    }
}
