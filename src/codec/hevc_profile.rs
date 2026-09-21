//! H.265 7.3.3 profile_tier_level syntax, including sublayer presence flags.
use super::bits::BitReader;
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub space: u8,
    pub tier: bool,
    pub idc: u8,
    /// Bit 31 is profile_compatibility_flag[0].
    pub compatibility: u32,
    /// The complete 48-bit constraint field, including source flags and inbld.
    pub constraints: u64,
}
impl Profile {
    pub fn compatible_with(self, idc: u8) -> bool {
        idc < 32 && (self.idc == idc || self.compatibility & (1 << (31 - idc)) != 0)
    }
    fn read(bits: &mut BitReader<'_>) -> Result<Self> {
        let profile = Self {
            space: bits.read(2)? as u8,
            tier: bits.bit()?,
            idc: bits.read(5)? as u8,
            compatibility: bits.read(32)?,
            constraints: (u64::from(bits.read(16)?) << 32) | u64::from(bits.read(32)?),
        };
        let compatible = |ids: &[u8]| ids.iter().any(|&id| profile.compatible_with(id));
        let low = |count: u32| (1u64 << count) - 1;
        let reserved = if compatible(&[4, 5, 6, 7, 8, 9, 10, 11]) {
            low(if compatible(&[5, 9, 10, 11]) { 33 } else { 34 }) << 1
        } else if profile.compatible_with(2) {
            (low(7) << 37) | (low(35) << 1)
        } else {
            low(43) << 1
        };
        let reserved = reserved | u64::from(!compatible(&[1, 2, 3, 4, 5, 9, 11]));
        if profile.constraints & reserved != 0 {
            return Err(invalid("nonzero HEVC profile reserved constraint bits"));
        }
        Ok(profile)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubLayer {
    /// None means this syntax structure does not signal a profile/level here.
    pub profile: Option<Profile>,
    pub level: Option<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileTierLevel {
    pub profile: Option<Profile>,
    pub level: u8,
    pub sub_layers: Vec<SubLayer>,
}
impl ProfileTierLevel {
    pub fn read(
        bits: &mut BitReader<'_>,
        profile_present: bool,
        max_sub_layers_minus1: u8,
    ) -> Result<Self> {
        if max_sub_layers_minus1 > 6 {
            return Err(invalid("HEVC sublayer count exceeds seven"));
        }
        let profile = if profile_present {
            Some(Profile::read(bits)?)
        } else {
            None
        };
        let level = bits.read(8)? as u8;
        let mut presence = [(false, false); 6];
        for entry in presence.iter_mut().take(max_sub_layers_minus1 as usize) {
            *entry = (bits.bit()?, bits.bit()?);
        }
        if max_sub_layers_minus1 > 0 {
            for _ in max_sub_layers_minus1..8 {
                if bits.read(2)? != 0 {
                    return Err(invalid("nonzero HEVC sublayer reserved bits"));
                }
            }
        }
        let mut sub_layers = Vec::with_capacity(max_sub_layers_minus1 as usize);
        for (has_profile, has_level) in presence.into_iter().take(max_sub_layers_minus1 as usize) {
            sub_layers.push(SubLayer {
                profile: if has_profile {
                    Some(Profile::read(bits)?)
                } else {
                    None
                },
                level: if has_level {
                    Some(bits.read(8)? as u8)
                } else {
                    None
                },
            });
        }
        Ok(Self {
            profile,
            level,
            sub_layers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_x265_vps_and_sps_agree_and_leave_next_fields_aligned() {
        use super::super::hevc_nal::NalRbsp;
        let mut profiles = Vec::new();
        for (hex, prefix) in [
            ("40010c01ffff01600000030090000003000003001e959809", 32),
            (
                "42010101600000030090000003000003001ea020810596566924caf0168080000003008000000c84",
                8,
            ),
        ] {
            let nal: Vec<_> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let rbsp = NalRbsp::parse(&nal, 1024).unwrap();
            let mut bits = BitReader::new(&rbsp.bytes);
            bits.skip(prefix).unwrap();
            let p = ProfileTierLevel::read(&mut bits, true, 0).unwrap();
            assert_eq!(p.level, 30);
            let profile = p.profile.unwrap();
            assert_eq!(profile.idc, 1);
            assert_eq!(profile.compatibility, 0x60000000);
            assert_eq!(profile.constraints, 0x900000000000);
            assert_eq!(bits.position(), prefix + 96);
            if rbsp.header.unit_type == 33 {
                assert_eq!(bits.unsigned_golomb().unwrap(), 0); // SPS ID
                assert_eq!(bits.unsigned_golomb().unwrap(), 1); // 4:2:0
                assert_eq!(bits.unsigned_golomb().unwrap(), 64);
                assert_eq!(bits.unsigned_golomb().unwrap(), 64);
            }
            profiles.push(p);
        }
        assert_eq!(profiles[0], profiles[1]);
    }
    #[test]
    fn profile_constraints_and_sublayer_presence() {
        // Main10: source flags 1001, one-picture-only constraint, inbld.
        let mut bytes = vec![2, 0x20, 0, 0, 0, 0x90, 0x10, 0, 0, 0, 1, 120];
        let parsed = ProfileTierLevel::read(&mut BitReader::new(&bytes), true, 0).unwrap();
        assert_eq!(parsed.level, 120);
        assert!(parsed.profile.unwrap().compatible_with(2));
        assert!(!parsed.profile.unwrap().compatible_with(1));
        for n in 0..bytes.len() {
            assert!(ProfileTierLevel::read(&mut BitReader::new(&bytes[..n]), true, 0).is_err());
        }
        bytes[8] = 1;
        assert!(ProfileTierLevel::read(&mut BitReader::new(&bytes), true, 0).is_err());
        // No general profile; sublayer0 level only, sublayer1 absent.
        let bytes = [120, 0x40, 0, 90, 0x80];
        let mut bits = BitReader::new(&bytes);
        let p = ProfileTierLevel::read(&mut bits, false, 2).unwrap();
        assert_eq!(p.profile, None);
        assert_eq!(
            p.sub_layers,
            vec![
                SubLayer {
                    profile: None,
                    level: Some(90)
                },
                SubLayer::default()
            ]
        );
        bits.finish_rbsp().unwrap();
        assert!(
            ProfileTierLevel::read(&mut BitReader::new(&[120, 0x41, 0, 90]), false, 2).is_err()
        );
        assert!(ProfileTierLevel::read(&mut BitReader::new(&[]), false, 7).is_err());
    }
}
