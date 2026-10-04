//! H.265 Annex E.2.1 video usability information, preserved for output scheduling.
use super::{bits::BitReader, hevc_hrd::Hrd, hevc_vps::Timing};
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AspectRatio {
    pub idc: u8,
    pub extended: Option<[u16; 2]>,
}
impl AspectRatio {
    pub fn ratio(self) -> Option<(u16, u16)> {
        const SAR: [(u16, u16); 16] = [
            (1, 1),
            (12, 11),
            (10, 11),
            (16, 11),
            (40, 33),
            (24, 11),
            (20, 11),
            (32, 11),
            (80, 33),
            (18, 11),
            (15, 11),
            (64, 33),
            (160, 99),
            (4, 3),
            (3, 2),
            (2, 1),
        ];
        let ratio = if self.idc == 255 {
            self.extended.map(|[x, y]| (x, y))
        } else {
            self.idc
                .checked_sub(1)
                .and_then(|index| SAR.get(index as usize).copied())
        };
        ratio.filter(|(x, y)| *x != 0 && *y != 0)
    }
}
#[cfg(test)]
mod aspect_tests {
    use super::AspectRatio;
    #[test]
    fn standard_extended_and_unspecified_sample_aspects_are_distinct() {
        let ratio = |idc| {
            AspectRatio {
                idc,
                extended: None,
            }
            .ratio()
        };
        assert_eq!(ratio(2), Some((12, 11)));
        assert_eq!(ratio(14), Some((4, 3)));
        assert_eq!(ratio(16), Some((2, 1)));
        assert_eq!(ratio(0), None);
        assert_eq!(ratio(17), None);
        assert_eq!(
            AspectRatio {
                idc: 255,
                extended: Some([7, 5])
            }
            .ratio(),
            Some((7, 5))
        );
        assert_eq!(
            AspectRatio {
                idc: 255,
                extended: Some([0, 5])
            }
            .ratio(),
            None
        );
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSignal {
    pub format: u8,
    pub full_range: bool,
    /// Primaries, transfer characteristics and matrix coefficients.
    pub colour: Option<[u8; 3]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Restriction {
    pub tiles_fixed: bool,
    pub motion_over_boundaries: bool,
    pub restricted_reference_lists: bool,
    pub min_spatial_segmentation: u16,
    pub max_bytes_per_picture_denom: u8,
    pub max_bits_per_min_cu_denom: u8,
    pub max_mv_log2: [u8; 2],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vui {
    pub aspect_ratio: Option<AspectRatio>,
    pub overscan_appropriate: Option<bool>,
    pub signal: Option<VideoSignal>,
    pub chroma_location: Option<[u8; 2]>,
    pub neutral_chroma: bool,
    pub field_sequence: bool,
    pub frame_field_info: bool,
    /// Left/right/top/bottom in chroma-dependent display-window units.
    pub display_window: Option<[u32; 4]>,
    pub timing: Option<Timing>,
    pub hrd: Option<Hrd>,
    pub restriction: Option<Restriction>,
}
fn bounded(b: &mut BitReader<'_>, max: u32) -> Result<u32> {
    let value = b.unsigned_golomb()?;
    if value > max {
        return Err(invalid("HEVC VUI value exceeds range"));
    }
    Ok(value)
}
impl Vui {
    pub fn read(b: &mut BitReader<'_>, max_sub_layers_minus1: u8) -> Result<Self> {
        if max_sub_layers_minus1 > 6 {
            return Err(invalid("invalid VUI sublayer count"));
        }
        let aspect_ratio = if b.bit()? {
            let idc = b.read(8)? as u8;
            let extended = if idc == 255 {
                Some([b.read(16)? as u16, b.read(16)? as u16])
            } else {
                None
            };
            Some(AspectRatio { idc, extended })
        } else {
            None
        };
        let overscan_appropriate = if b.bit()? { Some(b.bit()?) } else { None };
        let signal = if b.bit()? {
            Some(VideoSignal {
                format: b.read(3)? as u8,
                full_range: b.bit()?,
                colour: if b.bit()? {
                    Some([b.read(8)? as u8, b.read(8)? as u8, b.read(8)? as u8])
                } else {
                    None
                },
            })
        } else {
            None
        };
        let chroma_location = if b.bit()? {
            Some([bounded(b, 5)? as u8, bounded(b, 5)? as u8])
        } else {
            None
        };
        let neutral_chroma = b.bit()?;
        let field_sequence = b.bit()?;
        let frame_field_info = b.bit()?;
        let display_window = if b.bit()? {
            Some([
                b.unsigned_golomb()?,
                b.unsigned_golomb()?,
                b.unsigned_golomb()?,
                b.unsigned_golomb()?,
            ])
        } else {
            None
        };
        let mut hrd = None;
        let timing = if b.bit()? {
            let units_in_tick = b.read(32)?;
            let time_scale = b.read(32)?;
            if units_in_tick == 0 || time_scale == 0 {
                return Err(invalid("zero HEVC VUI clock"));
            }
            let ticks_per_poc = if b.bit()? {
                Some(u64::from(bounded(b, u32::MAX - 1)?) + 1)
            } else {
                None
            };
            if b.bit()? {
                hrd = Some(Hrd::read(b, max_sub_layers_minus1, None)?);
            }
            Some(Timing {
                units_in_tick,
                time_scale,
                ticks_per_poc,
            })
        } else {
            None
        };
        let restriction = if b.bit()? {
            Some(Restriction {
                tiles_fixed: b.bit()?,
                motion_over_boundaries: b.bit()?,
                restricted_reference_lists: b.bit()?,
                min_spatial_segmentation: bounded(b, 4095)? as u16,
                max_bytes_per_picture_denom: bounded(b, 16)? as u8,
                max_bits_per_min_cu_denom: bounded(b, 16)? as u8,
                max_mv_log2: [bounded(b, 15)? as u8, bounded(b, 15)? as u8],
            })
        } else {
            None
        };
        Ok(Self {
            aspect_ratio,
            overscan_appropriate,
            signal,
            chroma_location,
            neutral_chroma,
            field_sequence,
            frame_field_info,
            display_window,
            timing,
            hrd,
            restriction,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_colour_window_and_restrictions_survive_parsing() {
        let data = [
            255, 128, 2, 0, 1, 251, 128, 134, 128, 217, 45, 50, 22, 0, 0, 7, 210, 0, 1, 212, 193,
            78, 0, 8, 0, 4, 66, 33, 0, 132,
        ];
        let mut bits = BitReader::new(&data);
        let v = Vui::read(&mut bits, 2).unwrap();
        assert_eq!(v.aspect_ratio.unwrap().extended, Some([4, 3]));
        assert_eq!(v.overscan_appropriate, Some(true));
        assert_eq!(
            v.signal,
            Some(VideoSignal {
                format: 5,
                full_range: true,
                colour: Some([1, 13, 1])
            })
        );
        assert_eq!(v.chroma_location, Some([2, 3]));
        assert!(v.neutral_chroma && v.frame_field_info && !v.field_sequence);
        assert_eq!(v.display_window, Some([1, 2, 3, 4]));
        assert_eq!(
            v.timing,
            Some(Timing {
                units_in_tick: 1001,
                time_scale: 60000,
                ticks_per_poc: Some(2)
            })
        );
        let r = v.restriction.unwrap();
        assert_eq!(r.min_spatial_segmentation, 4095);
        assert_eq!(r.max_bytes_per_picture_denom, 16);
        assert_eq!(r.max_bits_per_min_cu_denom, 16);
        assert_eq!(r.max_mv_log2, [15, 15]);
        assert!(r.tiles_fixed && r.motion_over_boundaries && !r.restricted_reference_lists);
        bits.finish_rbsp().unwrap();
        for length in 0..30 {
            assert!(Vui::read(&mut BitReader::new(&data[..length]), 2).is_err());
        }
    }
    #[test]
    fn real_sps_vui_timing_and_end_alignment() {
        let hex =
            "42010101600000030090000003000003001ea020810596566924caf0168080000003008000000c84";
        let nal: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let rbsp = super::super::hevc_nal::NalRbsp::parse(&nal, 1024).unwrap();
        let mut bits = BitReader::new(&rbsp.bytes);
        // trace_headers oracle locates VUI at bit195 including the 16-bit NAL header.
        bits.skip(179).unwrap();
        let vui = Vui::read(&mut bits, 0).unwrap();
        assert_eq!(
            vui.aspect_ratio,
            Some(AspectRatio {
                idc: 1,
                extended: None
            })
        );
        assert_eq!(
            vui.timing,
            Some(Timing {
                units_in_tick: 1,
                time_scale: 25,
                ticks_per_poc: None
            })
        );
        assert_eq!(vui.hrd, None);
        assert_eq!(vui.restriction, None);
        assert_eq!(bits.position(), 268);
        assert!(!bits.bit().unwrap()); // sps_extension_present_flag
        bits.finish_rbsp().unwrap();
    }
}
