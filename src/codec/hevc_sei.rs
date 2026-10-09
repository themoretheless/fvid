//! H.265 D.2.2 SEI messages: the payload-type/payload-size walk over an RBSP.
//!
//! An SEI NAL unit is a sequence of messages, each announced by a type and a
//! size written as a run of `0xFF` bytes followed by the remainder, so a
//! decoder can pass over the messages it does not implement. The two static
//! HDR messages carry the payloads [`crate::color::hdr`] reads: SEI 137 holds
//! the mastering display volume and SEI 144 the content light levels, in the
//! same 24 and 4 bytes an MP4 `mdcv`/`ccll` box holds.
use super::hevc_nal::NalRbsp;
use crate::color::hdr::{HdrMetadata, SEI_CLLI, SEI_MDCV};
use crate::{Result, invalid};

/// Prefix SEI, carried before the coded picture it describes.
pub const NAL_UNIT_PREFIX_SEI: u8 = 39;
/// Suffix SEI, carried after it.
pub const NAL_UNIT_SUFFIX_SEI: u8 = 40;

pub fn is_sei_unit(unit_type: u8) -> bool {
    matches!(unit_type, NAL_UNIT_PREFIX_SEI | NAL_UNIT_SUFFIX_SEI)
}

/// Encode explicitly supplied output HDR metadata as a base-layer prefix SEI
/// NAL, including RBSP trailing bits and emulation-prevention bytes.
pub fn output_hdr_nal(hdr: &HdrMetadata) -> Result<Vec<u8>> {
    let mut rbsp = Vec::new();
    if let Some(display) = hdr.mastering {
        if display.min_luminance > display.max_luminance {
            return Err(invalid(
                "output mastering minimum exceeds maximum luminance",
            ));
        }
        let points = [display.red, display.green, display.blue, display.white];
        if points.iter().any(|p| {
            !p.x.is_finite()
                || !p.y.is_finite()
                || !(0.0..=1.0).contains(&p.x)
                || !(0.0..=1.0).contains(&p.y)
        }) || [display.max_luminance, display.min_luminance]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0 || f64::from(*v) * 10000.0 > f64::from(u32::MAX))
        {
            return Err(invalid(
                "output mastering display exceeds SEI representation",
            ));
        }
        let payload = crate::color::hdr::mdcv_payload(&display);
        if crate::color::hdr::MasteringDisplay::from_payload(&payload).is_none() {
            return Err(invalid("invalid output mastering display"));
        }
        rbsp.extend_from_slice(&[SEI_MDCV, payload.len() as u8]);
        rbsp.extend_from_slice(&payload);
    }
    let light = [hdr.light.max_cll, hdr.light.max_fall];
    if light
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0 || *v > 65535.0 || v.fract() != 0.0)
    {
        return Err(invalid(
            "output content light levels must be integer nits in 0..65535",
        ));
    }
    if light != [0.0, 0.0] {
        rbsp.extend_from_slice(&[SEI_CLLI, 4]);
        for value in light {
            rbsp.extend_from_slice(&(value as u16).to_be_bytes());
        }
    }
    if rbsp.is_empty() {
        return Ok(Vec::new());
    }
    rbsp.push(0x80);
    let mut nal = vec![NAL_UNIT_PREFIX_SEI << 1, 1];
    let mut zeros = 0;
    for byte in rbsp {
        if zeros == 2 && byte <= 3 {
            nal.push(3);
            zeros = 0;
        }
        nal.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    Ok(nal)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message<'a> {
    pub payload_type: u32,
    pub payload: &'a [u8],
}

/// A `payloadType` or `payloadSize`, extended by a run of `0xFF` bytes.
fn code(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    let mut sum = 0u32;
    while let Some(&byte) = bytes.get(*cursor) {
        *cursor += 1;
        if byte != 0xff {
            return Ok(sum + u32::from(byte));
        }
        sum += u32::from(byte);
    }
    Err(invalid("SEI message field ends inside a 0xFF run"))
}

/// The padding that follows the last message: an `0x80` marker bit, then zeros.
/// A zero-length message of type 128 would look the same; none is in use.
fn is_trailing(bytes: &[u8]) -> bool {
    bytes.first() == Some(&0x80) && bytes[1..].iter().all(|&byte| byte == 0)
}

/// Every message in an SEI RBSP, unknown payloads returned unread.
pub fn messages(rbsp: &[u8]) -> Result<Vec<Message<'_>>> {
    let mut found = Vec::new();
    let mut cursor = 0;
    loop {
        let rest = &rbsp[cursor..];
        if rest.is_empty() || is_trailing(rest) {
            return Ok(found);
        }
        let payload_type = code(rbsp, &mut cursor)?;
        let size = code(rbsp, &mut cursor)?;
        let size: usize = size
            .try_into()
            .map_err(|_| invalid("SEI payload size too large"))?;
        let end = cursor + size;
        let payload = rbsp
            .get(cursor..end)
            .ok_or_else(|| invalid("SEI payload runs past the RBSP"))?;
        found.push(Message {
            payload_type,
            payload,
        });
        cursor = end;
    }
}

/// Base-layer active_parameter_sets (payload type 129), H.265 D.2.21.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveParameterSets {
    pub vps_id: u8,
    pub self_contained_cvs: bool,
    pub no_parameter_set_update: bool,
    /// Only the first SPS is activated for base-layer Annex A decoding.
    /// Additional IDs are bounded and preserved as reserved guidance.
    pub sps_ids: Vec<u8>,
}
impl ActiveParameterSets {
    pub fn parse(payload: &[u8]) -> Result<Self> {
        let mut bits = super::bits::BitReader::new(payload);
        let vps_id = bits.read(4)? as u8;
        let self_contained_cvs = bits.bit()?;
        let no_parameter_set_update = bits.bit()?;
        let count = bits.unsigned_golomb()?;
        if count > 15 {
            return Err(invalid("HEVC active parameter SEI exceeds SPS count"));
        }
        let mut sps_ids = Vec::with_capacity(count as usize + 1);
        for _ in 0..=count {
            let id = bits.unsigned_golomb()?;
            if id > 15 {
                return Err(invalid("HEVC active parameter SEI SPS ID outside range"));
            }
            sps_ids.push(id as u8);
        }
        // H.265 D.3.1 requires ignoring reserved payload-extension data.
        // Retain the known fields and validate the final marker/alignment;
        // syntax ending exactly at the payload boundary needs no marker.
        if bits.remaining() != 0 {
            while bits.more_rbsp_data() {
                bits.bit()?;
            }
            bits.finish_rbsp()?;
        }
        Ok(Self { vps_id, self_contained_cvs, no_parameter_set_update, sps_ids })
    }
}

/// Read active-parameter guidance from a base-layer prefix SEI. Other messages
/// remain opaque; malformed guidance is reported to callers separately from HDR.
pub fn active_parameters_from_nal(nal: &[u8], budget: usize) -> Result<Option<ActiveParameterSets>> {
    let rbsp = NalRbsp::parse(nal, budget)?;
    if rbsp.header.unit_type != NAL_UNIT_PREFIX_SEI {
        return Ok(None);
    }
    rbsp.header.require_base_layer()?;
    let mut active = None;
    let found = messages(&rbsp.bytes)?;
    for message in &found {
        if message.payload_type == 129 {
            if found.len() != 1 {
                return Err(invalid("HEVC active parameter SEI must occupy its own NAL"));
            }
            active = Some(ActiveParameterSets::parse(message.payload)?);
        }
    }
    Ok(active)
}

/// The static HDR metadata an SEI RBSP states, message by message.
///
/// A malformed 137 or 144 payload is dropped rather than failed: the metadata
/// only guides tone mapping, so it must not cost the picture it describes.
pub fn hdr_from_rbsp(rbsp: &[u8]) -> Result<HdrMetadata> {
    let mut hdr = HdrMetadata::default();
    for message in messages(rbsp)? {
        let decoded = if message.payload_type == u32::from(SEI_MDCV) {
            HdrMetadata::from_mdcv(message.payload)
        } else if message.payload_type == u32::from(SEI_CLLI) {
            HdrMetadata::from_clli(message.payload)
        } else {
            None
        };
        if let Some(decoded) = decoded {
            hdr.merge(decoded);
        }
    }
    Ok(hdr)
}

/// Static HDR metadata from one NAL unit, or `None` when it states none.
pub fn hdr_from_nal(nal: &[u8], budget: usize) -> Result<Option<HdrMetadata>> {
    let rbsp = NalRbsp::parse(nal, budget)?;
    if !is_sei_unit(rbsp.header.unit_type) {
        return Ok(None);
    }
    rbsp.header.require_base_layer()?;
    Ok(Some(hdr_from_rbsp(&rbsp.bytes)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_parameter_payload_bounds_flags_and_alignment() {
        fn payload(count: u32, ids: &[u32]) -> Vec<u8> {
            fn ue(v: u32) -> String {
                let word = format!("{:b}", v + 1);
                "0".repeat(word.len() - 1) + &word
            }
            let mut bits = String::from("111111");
            bits += &ue(count);
            for &id in ids { bits += &ue(id); }
            if bits.len() % 8 != 0 {
                bits.push('1');
                while bits.len() % 8 != 0 { bits.push('0'); }
            }
            bits.as_bytes().chunks(8).map(|c| c.iter().fold(0, |v, &b| (v << 1) | u8::from(b == b'1'))).collect()
        }
        let data = payload(0, &[15]);
        let active = ActiveParameterSets::parse(&data).unwrap();
        assert_eq!(active.vps_id, 15);
        assert_eq!(active.sps_ids, [15]);
        assert!(active.self_contained_cvs && active.no_parameter_set_update);
        let data = payload(15, &[15; 16]);
        assert_eq!(ActiveParameterSets::parse(&data).unwrap().sps_ids, [15; 16]);
        assert_eq!(ActiveParameterSets::parse(&payload(16, &[])).unwrap_err().to_string(), "HEVC active parameter SEI exceeds SPS count");
        assert_eq!(ActiveParameterSets::parse(&payload(0, &[16])).unwrap_err().to_string(), "HEVC active parameter SEI SPS ID outside range");
        assert!(ActiveParameterSets::parse(&[]).is_err());
        let original = payload(0, &[15]);
        let expected = ActiveParameterSets::parse(&original).unwrap();
        let mut extended = original.clone();
        extended.extend_from_slice(&[0xab, 0xcd, 128]);
        assert_eq!(ActiveParameterSets::parse(&extended).unwrap(), expected);
        let mut no_stop = original;
        no_stop.extend_from_slice(&[0, 0, 0]);
        assert_eq!(ActiveParameterSets::parse(&no_stop).unwrap_err().to_string(),
            "truncated or oversized bit field");
        let mixed = sei(&[129, 1, 7, 5, 0, 128]);
        assert_eq!(active_parameters_from_nal(&mixed, 1024).unwrap_err().to_string(),
            "HEVC active parameter SEI must occupy its own NAL");
        for end in 0..data.len() - 1 {
            assert!(ActiveParameterSets::parse(&data[..end]).is_err());
        }
    }
    #[test]
    fn output_metadata_roundtrips_escaped_rbsp_and_rejects_invalid_light() {
        let hdr = HdrMetadata {
            mastering: Some(
                crate::color::hdr::MasteringDisplay::from_corners(
                    (0.68, 0.32),
                    (0.265, 0.69),
                    (0.15, 0.06),
                    (0.3127, 0.329),
                    1000.0,
                    0.005,
                )
                .unwrap(),
            ),
            light: crate::color::tonemap::ContentLight {
                max_cll: 1000.0,
                max_fall: 400.0,
            },
        };
        let nal = output_hdr_nal(&hdr).unwrap();
        assert!(nal.windows(3).any(|p| p == [0, 0, 3]));
        let decoded = hdr_from_nal(&nal, 4096).unwrap().unwrap();
        assert_eq!(decoded.light, hdr.light);
        assert_eq!(
            crate::color::hdr::mdcv_payload(&decoded.mastering.unwrap()),
            crate::color::hdr::mdcv_payload(&hdr.mastering.unwrap())
        );
        assert!(output_hdr_nal(&HdrMetadata::default()).unwrap().is_empty());
        for value in [f32::NAN, f32::INFINITY, -1.0, 65536.0, 1.5] {
            let bad = HdrMetadata {
                light: crate::color::tonemap::ContentLight {
                    max_cll: value,
                    max_fall: 0.0,
                },
                ..Default::default()
            };
            assert!(output_hdr_nal(&bad).is_err());
        }
    }

    /// An SEI NAL header plus the messages written after it.
    fn sei(messages: &[u8]) -> Vec<u8> {
        let mut nal = vec![0x4e, 0x01];
        nal.extend_from_slice(messages);
        nal
    }

    /// The two HDR messages x265 wrote for a BT.2020/PQ encode of a
    /// `master-display=G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L(10000000,1)
    /// max-cll=1234,567` stream, taken from the container's own bitstream.
    const MDCV: &str = "4e01891821349baa199608fc8a4839083d13404200989680000003000180";
    const CLLI: &str = "4e01900404d2023780";
    /// The same volume with the encoder's emulation-prevention byte removed:
    /// four `00` bytes in the minimum luminance need one `03` to travel.
    const MDCV_PAYLOAD: &str = "21349baa199608fc8a4839083d1340420098968000000001";

    fn hex(bytes: &str) -> Vec<u8> {
        (0..bytes.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&bytes[i..i + 2], 16).unwrap())
            .collect()
    }

    /// The copy an encoder makes of a payload that carries start-code patterns:
    /// a third `00` inserts a `03`, which is why the travelling bytes and the
    /// payload the reader sees differ in length.
    fn escape(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut zeros = 0;
        for &byte in bytes {
            if zeros == 2 && byte <= 3 {
                out.push(0x03);
                zeros = 0;
            }
            out.push(byte);
            zeros = if byte == 0 { zeros + 1 } else { 0 };
        }
        out
    }

    #[test]
    fn a_field_of_0ff_runs_adds_up_to_one_number() {
        let bytes = [0xffu8, 0xff, 0x01, 0x00];
        let mut cursor = 0;
        assert_eq!(code(&bytes, &mut cursor).unwrap(), 511);
        assert_eq!(cursor, 3);
        assert_eq!(code(&bytes[3..], &mut 0).unwrap(), 0);
        // A run that never ends states no number at all.
        assert!(code(&[0xff, 0xff], &mut 0).is_err());
    }

    #[test]
    fn messages_are_read_by_type_and_size_and_a_known_payload_is_not_consumed() {
        // A type of 0xFF + 0x41 is 320, and an empty payload travels as one
        // size byte.
        let rbsp = [
            0x05, 0x03, 0xaa, 0xbb, 0xcc, // type 5, three bytes
            0x81, 0x00, // type 129, empty
            0xff, 0x41, 0x01, 0x7f, 0x80, // type 320, one byte, then trailing bits
        ];
        let found = messages(&rbsp).unwrap();
        assert_eq!(
            found,
            vec![
                Message {
                    payload_type: 5,
                    payload: &[0xaa, 0xbb, 0xcc]
                },
                Message {
                    payload_type: 129,
                    payload: &[]
                },
                Message {
                    payload_type: 320,
                    payload: &[0x7f]
                },
            ]
        );
    }

    #[test]
    fn trailing_bits_end_the_walk_and_a_truncated_payload_does_not() {
        assert_eq!(messages(&[0x05, 0x01, 0x7f, 0x80]).unwrap().len(), 1);
        assert!(messages(&[]).unwrap().is_empty());
        assert!(messages(&[0x80]).unwrap().is_empty());
        let truncated: [&[u8]; 4] = [&[0x05], &[0x05, 0x01], &[0x05, 0x04, 0x7f, 0x80], &[0xff]];
        for bytes in truncated {
            assert!(messages(bytes).is_err(), "{bytes:x?} is not a message list");
        }
    }

    #[test]
    fn a_real_mastering_display_message_describes_the_volume_it_names() {
        let nal = hex(MDCV);
        let rbsp = NalRbsp::parse(&nal, 1 << 20).unwrap();
        assert_eq!(rbsp.header.unit_type, NAL_UNIT_PREFIX_SEI);
        let found = messages(&rbsp.bytes).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].payload_type, u32::from(SEI_MDCV));
        // The emulation-prevention byte the encoder inserted inside the
        // luminance field leaves a payload of the size the standard fixes.
        assert_eq!(found[0].payload.len(), 24);
        assert_eq!(found[0].payload, hex(MDCV_PAYLOAD).as_slice());
        let hdr = hdr_from_rbsp(&rbsp.bytes).unwrap();
        let display = hdr.mastering.unwrap();
        // Back to the encoder's own integers: the corners land where they are
        // named, not in the order the payload carries them.
        let unit = |c: f64| (c / 0.000_02).round();
        assert_eq!(
            [
                (unit(display.green.x), unit(display.green.y)),
                (unit(display.blue.x), unit(display.blue.y)),
                (unit(display.red.x), unit(display.red.y)),
                (unit(display.white.x), unit(display.white.y)),
            ],
            [
                (8500.0, 39_850.0),
                (6550.0, 2300.0),
                (35_400.0, 14_600.0),
                (15_635.0, 16_450.0),
            ]
        );
        assert!((display.max_luminance / 0.000_1).round() == 10_000_000.0);
        assert!((display.min_luminance / 0.000_1).round() == 1.0);
        assert!(display.is_hdr10());
        assert!(hdr.light.max_cll == 0.0);
        // The escape rule above is the encoder's own: it rebuilds this unit.
        assert_eq!(
            sei(&[
                &[SEI_MDCV, 0x18][..],
                &escape(&hex(MDCV_PAYLOAD)),
                &[0x80][..]
            ]
            .concat()),
            hex(MDCV)
        );
    }

    #[test]
    fn a_real_light_level_message_states_the_lights_it_names() {
        let nal = hex(CLLI);
        let hdr = hdr_from_nal(&nal, 1 << 20).unwrap().unwrap();
        assert_eq!(hdr.light.max_cll, 1234.0);
        assert_eq!(hdr.light.max_fall, 567.0);
        assert!(hdr.mastering.is_none());
    }

    #[test]
    fn the_two_halves_of_one_stream_add_up_to_the_whole_volume() {
        let mut hdr = hdr_from_nal(&hex(MDCV), 1 << 20).unwrap().unwrap();
        assert!(hdr.mastering.unwrap().is_hdr10());
        assert!(hdr.light.max_cll == 0.0 && hdr.light.max_fall == 0.0);
        hdr.merge(hdr_from_nal(&hex(CLLI), 1 << 20).unwrap().unwrap());
        assert_eq!(hdr.light.max_cll, 1234.0);
        assert_eq!(hdr.light.max_fall, 567.0);
        assert_eq!(hdr.content_light(1000.0).max_cll, 1234.0);
        // A volume names the panel it was graded on, to within the payload's
        // own quantization step; only a VUI or a colr box names the curve.
        assert!(hdr.mastering.unwrap().is_hdr10());
    }

    #[test]
    fn an_unread_message_is_passed_over_and_leaves_no_metadata() {
        // x265 writes its options as one user-data message, in the form the
        // encoder really used: a size of nine 0xFF bytes plus 0x51 = 2 376.
        let mut rbsp = vec![0x05u8];
        rbsp.extend_from_slice(&[0xff; 9]);
        rbsp.push(0x51);
        rbsp.extend_from_slice(&[0x2c; 2376]);
        rbsp.push(0x80);
        let found = messages(&rbsp).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].payload_type, found[0].payload.len()), (5, 2_376));
        assert!(hdr_from_rbsp(&rbsp).unwrap().is_empty());
        // A header whose payload never arrives is not a message list.
        assert!(messages(&rbsp[..11]).is_err());
    }

    #[test]
    fn only_sei_units_are_walked_and_a_multilayer_one_is_refused() {
        let nal = hex(CLLI);
        assert!(is_sei_unit(
            NalRbsp::parse(&nal, 1 << 20).unwrap().header.unit_type
        ));
        // A VPS NAL states no picture metadata.
        assert!(
            hdr_from_nal(&hex("40010c01ffff0220"), 1 << 20)
                .unwrap()
                .is_none()
        );
        // The same payload under a header that names a non-base layer.
        let mut multilayer = nal;
        multilayer[0] |= 1;
        assert!(hdr_from_nal(&multilayer, 1 << 20).is_err());
        let mut suffix = hex(CLLI);
        suffix[0] = (NAL_UNIT_SUFFIX_SEI << 1) | (suffix[0] & 1);
        assert_eq!(
            hdr_from_nal(&suffix, 1 << 20)
                .unwrap()
                .unwrap()
                .light
                .max_cll,
            1234.0
        );
        // A payload budget stops the walk before it allocates for a big one.
        assert!(hdr_from_nal(&hex(MDCV), 8).is_err());
    }

    #[test]
    fn a_message_that_is_too_short_for_the_volume_it_claims_states_nothing() {
        let hdr = hdr_from_nal(&sei(&[SEI_MDCV, 0x02, 0x21, 0x34, 0x80]), 1 << 20)
            .unwrap()
            .unwrap();
        assert!(hdr.is_empty());
        let hdr = hdr_from_nal(&sei(&[SEI_CLLI, 0x02, 0x04, 0xd2, 0x80]), 1 << 20)
            .unwrap()
            .unwrap();
        assert!(hdr.is_empty());
    }

    #[test]
    fn one_unit_can_carry_both_halves_and_a_later_block_overrides_only_what_it_knows() {
        let volume = escape(&hex(MDCV_PAYLOAD));
        let both = sei(&[
            &[SEI_CLLI, 0x04, 0x04, 0xd2, 0x02, 0x37][..],
            &[SEI_MDCV, 0x18][..],
            &volume,
            &[0x80][..],
        ]
        .concat());
        let hdr = hdr_from_nal(&both, 1 << 20).unwrap().unwrap();
        assert!(hdr.mastering.unwrap().is_hdr10());
        assert_eq!(hdr.light.max_fall, 567.0);
        // A block that only knows the light level must not clear the volume.
        let mut volume = hdr;
        volume.merge(HdrMetadata::from_clli(&[0x01, 0x00, 0x00, 0x64]).unwrap());
        assert_eq!(volume.mastering, hdr.mastering);
        assert_eq!(
            (volume.light.max_cll, volume.light.max_fall),
            (256.0, 100.0)
        );
    }
}
