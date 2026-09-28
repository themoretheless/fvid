//! AV1 sequence headers, including timing, operating points and color configuration.
use super::bits::BitReader;
use crate::color::hdr::ColourDescription;
use crate::{Result, invalid};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timing {
    pub display_tick: u32,
    pub time_scale: u32,
    pub ticks_per_picture: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderModel {
    pub delay_bits: u8,
    pub decoding_tick: u32,
    pub removal_bits: u8,
    pub presentation_bits: u8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatingPoint {
    pub idc: u16,
    pub level: u8,
    pub tier: bool,
    pub delays: Option<(u32, u32, bool)>,
    pub initial_display_delay: Option<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Color {
    pub depth: u8,
    pub monochrome: bool,
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
    pub subsampling: [bool; 2],
    pub chroma_position: u8,
    pub separate_uv_delta_q: bool,
}
impl Color {
    /// The H.273 signal the sequence header states. AV1's own tables for
    /// primaries, transfer characteristics and matrix coefficients are the same
    /// code points ISO/IEC 23091-2 writes, so the three values and the range
    /// flag carry over as they are; a code this module has no curve for stays
    /// the code the stream said.
    pub fn signal(&self) -> ColourDescription {
        ColourDescription {
            primaries: self.primaries,
            transfer: self.transfer,
            matrix: self.matrix,
            full_range: self.full_range,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequence {
    pub profile: u8,
    pub still_picture: bool,
    pub reduced_header: bool,
    pub timing: Option<Timing>,
    pub decoder_model: Option<DecoderModel>,
    pub operating_points: Vec<OperatingPoint>,
    pub dimension_bits: [u8; 2],
    pub max_size: [u32; 2],
    pub frame_id_bits: Option<(u8, u8)>,
    pub superblock128: bool,
    pub filter_intra: bool,
    pub intra_edge_filter: bool,
    pub interintra_compound: bool,
    pub masked_compound: bool,
    pub warped_motion: bool,
    pub dual_filter: bool,
    pub order_hint_bits: u8,
    pub joint_compound: bool,
    pub reference_mvs: bool,
    /// 0/1 forces a value; 2 selects the value in each frame header.
    pub screen_content_tools: u8,
    pub integer_mv: u8,
    pub superres: bool,
    pub cdef: bool,
    pub restoration: bool,
    pub color: Color,
    pub film_grain: bool,
}
fn uvlc(b: &mut BitReader<'_>) -> Result<u32> {
    let mut leading = 0u8;
    while !b.bit()? {
        leading += 1;
        if leading == 32 {
            return Ok(u32::MAX);
        }
    }
    Ok(((1u32 << leading) - 1) + b.read(leading)?)
}
fn color(b: &mut BitReader<'_>, profile: u8) -> Result<Color> {
    let high = b.bit()?;
    let depth = if profile == 2 && high {
        if b.bit()? { 12 } else { 10 }
    } else if high {
        10
    } else {
        8
    };
    let monochrome = profile != 1 && b.bit()?;
    let (primaries, transfer, matrix) = if b.bit()? {
        (b.read(8)? as u8, b.read(8)? as u8, b.read(8)? as u8)
    } else {
        (2, 2, 2)
    };
    let mut chroma_position = 0;
    let (full_range, subsampling, separate_uv_delta_q) = if monochrome {
        (b.bit()?, [true, true], false)
    } else {
        let (range, sub) = if (primaries, transfer, matrix) == (1, 13, 0) {
            if profile == 0 || (profile == 2 && depth != 12) {
                return Err(invalid("AV1 RGB color configuration violates profile"));
            }
            (true, [false, false])
        } else {
            let range = b.bit()?;
            let sub = match profile {
                0 => [true, true],
                1 => [false, false],
                _ if depth == 12 => {
                    let x = b.bit()?;
                    [x, x && b.bit()?]
                }
                _ => [true, false],
            };
            if sub == [true, true] {
                chroma_position = b.read(2)? as u8;
                if chroma_position == 3 {
                    return Err(invalid("reserved AV1 chroma sample position"));
                }
            }
            if matrix == 0 && sub != [false, false] {
                return Err(invalid("AV1 identity matrix requires 4:4:4"));
            }
            (range, sub)
        };
        (range, sub, b.bit()?)
    };
    Ok(Color {
        depth,
        monochrome,
        primaries,
        transfer,
        matrix,
        full_range,
        subsampling,
        chroma_position,
        separate_uv_delta_q,
    })
}
impl Sequence {
    pub fn parse(payload: &[u8]) -> Result<Self> {
        let b = &mut BitReader::new(payload);
        let profile = b.read(3)? as u8;
        if profile > 2 {
            return Err(invalid("reserved AV1 sequence profile"));
        }
        let still_picture = b.bit()?;
        let reduced_header = b.bit()?;
        if reduced_header && !still_picture {
            return Err(invalid("AV1 reduced header requires still picture"));
        }
        let mut timing = None;
        let mut decoder_model: Option<DecoderModel> = None;
        let mut operating_points = Vec::new();
        if reduced_header {
            operating_points.push(OperatingPoint {
                idc: 0,
                level: b.read(5)? as u8,
                tier: false,
                delays: None,
                initial_display_delay: None,
            });
        } else {
            if b.bit()? {
                let display_tick = b.read(32)?;
                let time_scale = b.read(32)?;
                if display_tick == 0 || time_scale == 0 {
                    return Err(invalid("zero AV1 timing scale"));
                }
                let ticks_per_picture = if b.bit()? {
                    Some(u64::from(uvlc(b)?) + 1)
                } else {
                    None
                };
                timing = Some(Timing {
                    display_tick,
                    time_scale,
                    ticks_per_picture,
                });
                if b.bit()? {
                    let model = DecoderModel {
                        delay_bits: b.read(5)? as u8 + 1,
                        decoding_tick: b.read(32)?,
                        removal_bits: b.read(5)? as u8 + 1,
                        presentation_bits: b.read(5)? as u8 + 1,
                    };
                    if model.decoding_tick == 0 {
                        return Err(invalid("zero AV1 decoder tick"));
                    }
                    decoder_model = Some(model);
                }
            }
            let initial_delay = b.bit()?;
            let count = b.read(5)? + 1;
            for _ in 0..count {
                let idc = b.read(12)? as u16;
                let level = b.read(5)? as u8;
                let tier = level > 7 && b.bit()?;
                let delays = if let Some(model) = &decoder_model {
                    if b.bit()? {
                        Some((
                            b.read(model.delay_bits)?,
                            b.read(model.delay_bits)?,
                            b.bit()?,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let initial_display_delay = if initial_delay && b.bit()? {
                    Some(b.read(4)? as u8 + 1)
                } else {
                    None
                };
                operating_points.push(OperatingPoint {
                    idc,
                    level,
                    tier,
                    delays,
                    initial_display_delay,
                });
            }
        }
        let dimension_bits = [b.read(4)? as u8 + 1, b.read(4)? as u8 + 1];
        let max_size = [
            b.read(dimension_bits[0])? + 1,
            b.read(dimension_bits[1])? + 1,
        ];
        let frame_id_bits = if !reduced_header && b.bit()? {
            let delta = b.read(4)? as u8 + 2;
            let total = delta + b.read(3)? as u8 + 1;
            if total > 16 {
                return Err(invalid("AV1 frame ID exceeds 16 bits"));
            }
            Some((delta, total))
        } else {
            None
        };
        let superblock128 = b.bit()?;
        let filter_intra = b.bit()?;
        let intra_edge_filter = b.bit()?;
        let mut interintra_compound = false;
        let mut masked_compound = false;
        let mut warped_motion = false;
        let mut dual_filter = false;
        let mut order_hint_bits = 0;
        let mut joint_compound = false;
        let mut reference_mvs = false;
        let mut screen_content_tools = 2;
        let mut integer_mv = 2;
        if !reduced_header {
            interintra_compound = b.bit()?;
            masked_compound = b.bit()?;
            warped_motion = b.bit()?;
            dual_filter = b.bit()?;
            let order_hint = b.bit()?;
            if order_hint {
                joint_compound = b.bit()?;
                reference_mvs = b.bit()?;
            }
            if !b.bit()? {
                screen_content_tools = b.read(1)? as u8;
            }
            if screen_content_tools > 0 && !b.bit()? {
                integer_mv = b.read(1)? as u8;
            }
            if order_hint {
                order_hint_bits = b.read(3)? as u8 + 1;
            }
        }
        let superres = b.bit()?;
        let cdef = b.bit()?;
        let restoration = b.bit()?;
        let color = color(b, profile)?;
        let film_grain = b.bit()?;
        // AV1 trailing_bits consumes the entire OBU payload, not just byte alignment.
        if !b.bit()? {
            return Err(invalid("missing AV1 sequence trailing one bit"));
        }
        while b.remaining() > 0 {
            if b.bit()? {
                return Err(invalid("nonzero AV1 sequence trailing bits"));
            }
        }
        Ok(Self {
            profile,
            still_picture,
            reduced_header,
            timing,
            decoder_model,
            operating_points,
            dimension_bits,
            max_size,
            frame_id_bits,
            superblock128,
            filter_intra,
            intra_edge_filter,
            interintra_compound,
            masked_compound,
            warped_motion,
            dual_filter,
            order_hint_bits,
            joint_compound,
            reference_mvs,
            screen_content_tools,
            integer_mv,
            superres,
            cdef,
            restoration,
            color,
            film_grain,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::av1::Obus;
    #[test]
    fn svt_sequence_matches_independent_header_trace() {
        let data = include_bytes!("../../tests/fixtures/av1/sequence.obu");
        let obus = Obus::new(data).collect::<Result<Vec<_>>>().unwrap();
        let header = obus.iter().find(|o| o.kind == 1).unwrap();
        let s = Sequence::parse(header.payload).unwrap();
        assert_eq!(s.profile, 0);
        assert_eq!(s.max_size, [64, 64]);
        assert_eq!(s.dimension_bits, [6, 6]);
        assert_eq!(s.color.depth, 8);
        assert_eq!(s.color.subsampling, [true, true]);
        assert_eq!(s.order_hint_bits, 7);
        assert!(!s.filter_intra && s.intra_edge_filter && s.warped_motion);
        assert!(s.reference_mvs && s.cdef && !s.restoration && !s.film_grain);
        assert_eq!((s.screen_content_tools, s.integer_mv), (2, 2));
        for end in 0..header.payload.len() {
            assert!(
                Sequence::parse(&header.payload[..end]).is_err(),
                "prefix {end}"
            );
        }
        for i in 0..header.payload.len() {
            let mut mutated = header.payload.to_vec();
            mutated[i] ^= 0xff;
            let _ = Sequence::parse(&mutated);
        }
    }
    #[test]
    fn uvlc_saturates_at_32_zero_bits() {
        assert_eq!(uvlc(&mut BitReader::new(&[0; 4])).unwrap(), u32::MAX);
        assert_eq!(uvlc(&mut BitReader::new(&[0x80])).unwrap(), 0);
        assert_eq!(uvlc(&mut BitReader::new(&[0x60])).unwrap(), 2);
        assert!(uvlc(&mut BitReader::new(&[0; 3])).is_err());
    }
}
