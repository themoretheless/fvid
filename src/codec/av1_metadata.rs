//! AV1 5.8 metadata OBUs, read for the two static HDR payloads.
//!
//! The mastering display travels here in a different shape than in HEVC or
//! MP4, and the difference is not cosmetic: the AV1 spec's own semantics call
//! the chromaticities **0.16 fixed-point** and the luminances **24.8** and
//! **18.14 fixed-point** cd/m², where SMPTE ST 2086 — which SEI 137 and the
//! `mdcv` box follow — quantizes the same coordinates by 0.00002 and the same
//! luminances by 0.0001. AV1 also indexes its primaries **red, green, blue**,
//! while H.265 puts them in the order green, blue, red. Reading one standard's
//! bytes with the other's table gives a volume that is finite, in range and
//! wrong, so the two decoders are kept apart and each names its own units.
//!
//! Only MaxCLL and MaxFALL agree both ways: they are CEA-861.3 integers in
//! candelas per square meter in AV1 and in SEI 144 alike, which is why the
//! content light half is [`crate::color::hdr::HdrMetadata::from_clli`].
//!
//! Encoders also append one `rbsp_trailing_bits` byte after the payload, which
//! the syntax above does not ask for; dav1d refuses a metadata OBU without it.
//! Since every field here sits at a fixed offset from the type, that byte is
//! passed over rather than subtracted.
use super::av1::{Obu, Obus};
use crate::color::hdr::{HdrMetadata, MasteringDisplay};
use crate::invalid;
use crate::Result;

/// An OBU carrying a `metadata_type` and the syntax that type names.
pub const OBU_METADATA: u8 = 5;
/// `METADATA_TYPE_HDR_CLL`: content light levels.
pub const METADATA_HDR_CLL: u64 = 1;
/// `METADATA_TYPE_HDR_MDCV`: the mastering display colour volume.
pub const METADATA_HDR_MDCV: u64 = 2;

/// Length of the content light payload after its type: MaxCLL then MaxFALL.
pub const CLL_BODY_LEN: usize = 4;
/// Length of the volume payload after its type: three primaries and a white
/// point as 16-bit coordinates, then two 32-bit luminances.
pub const MDCV_BODY_LEN: usize = 24;

/// One big-endian field of `width` bytes, as its own fixed-point scale needs.
fn field(bytes: &[u8], at: usize, width: usize) -> u32 {
    match bytes.get(at..at + width) {
        Some(raw) => raw
            .iter()
            .fold(0u32, |value, &byte| (value << 8) | u32::from(byte)),
        None => 0,
    }
}

/// A `0.16` fixed-point chromaticity coordinate.
fn coordinate(raw: u32) -> f64 {
    f64::from(raw) / 65_536.0
}

/// A `metadata_type`, written as a LEB128 value, and the body that follows it.
pub fn metadata_type(payload: &[u8]) -> Result<(u64, &[u8])> {
    let mut value = 0u64;
    for i in 0..8 {
        let &byte = payload
            .get(i)
            .ok_or_else(|| invalid("truncated AV1 metadata type"))?;
        value |= u64::from(byte & 127) << (7 * i);
        if byte & 128 == 0 {
            return Ok((value, &payload[i + 1..]));
        }
    }
    Err(invalid("unterminated AV1 metadata type"))
}

/// The mastering display an AV1 volume payload states, in AV1's own units.
pub fn mastering_display(bytes: &[u8]) -> Option<MasteringDisplay> {
    if bytes.len() < MDCV_BODY_LEN {
        return None;
    }
    // The syntax indexes its primaries red, green, blue.
    let point = |at: usize| {
        (
            coordinate(field(bytes, at, 2)),
            coordinate(field(bytes, at + 2, 2)),
        )
    };
    MasteringDisplay::from_corners(
        point(0),
        point(4),
        point(8),
        point(12),
        field(bytes, 16, 4) as f32 / 256.0,
        field(bytes, 20, 4) as f32 / 16_384.0,
    )
}

/// Static HDR metadata from one metadata OBU payload, `None` for the other
/// metadata types and for a volume that states no display.
pub fn hdr_from_obu(obu: &Obu<'_>) -> Option<HdrMetadata> {
    if obu.kind != OBU_METADATA {
        return None;
    }
    let (kind, body) = metadata_type(obu.payload).ok()?;
    match kind {
        METADATA_HDR_CLL => HdrMetadata::from_clli(body),
        METADATA_HDR_MDCV => Some(HdrMetadata {
            mastering: mastering_display(body),
            light: Default::default(),
        }),
        _ => None,
    }
}

/// Static HDR metadata across every metadata OBU of one packet.
pub fn hdr_from_packet(packet: &[u8]) -> Result<HdrMetadata> {
    let mut hdr = HdrMetadata::default();
    for obu in Obus::new(packet) {
        if let Some(metadata) = hdr_from_obu(&obu?) {
            hdr.merge(metadata);
        }
    }
    Ok(hdr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::av1::Obus;

    /// A metadata OBU: type 5, no header extension, a size field, a LEB128
    /// metadata type and the body that type names.
    fn metadata_obu(kind: u64, body: &[u8]) -> Vec<u8> {
        let payload = std::iter::once(kind as u8)
            .chain(body.iter().copied())
            .collect::<Vec<u8>>();
        let mut packet = vec![0x2a, payload.len() as u8];
        packet.extend_from_slice(&payload);
        packet
    }

    /// An HDR10 volume as the AV1 syntax writes it: the BT.2020 corners and a
    /// D65 white as `0.16` coordinates, 1 000 cd/m² as `24.8` and 0.0001 cd/m²
    /// as `18.14`.
    fn hdr10_body() -> [u8; MDCV_BODY_LEN] {
        let coord = |c: f64| (c * 65_536.0).round() as u32;
        let mut body = [0u8; MDCV_BODY_LEN];
        let corners = [
            (0.708, 0.292),
            (0.170, 0.797),
            (0.131, 0.046),
            (0.3127, 0.3290),
        ];
        for (i, (x, y)) in corners.iter().enumerate() {
            body[i * 4..i * 4 + 2].copy_from_slice(&(coord(*x) as u16).to_be_bytes());
            body[i * 4 + 2..i * 4 + 4].copy_from_slice(&(coord(*y) as u16).to_be_bytes());
        }
        body[16..20].copy_from_slice(&((1000.0f64 * 256.0).round() as u32).to_be_bytes());
        body[20..24].copy_from_slice(&((0.0001f64 * 16_384.0).round() as u32).to_be_bytes());
        body
    }

    /// SVT-AV1 v4.2.0 writing `--content-light 1234,567`: the OBU header, a LEB128
    /// type of 1, two integers and a trailing bit byte.
    const CLL_OBU: &str = "2a060104d2023780";
    /// The same encoder writing
    /// `--mastering-display "G(0.170,0.797)B(0.131,0.046)R(0.708,0.292)WP(0.3127,0.3290)L(1000.0,0.0001)"`.
    /// The command line names green, blue and red; the payload does not.
    const MDCV_OBU: &str = "2a1a02b53f4ac12b85cc0821890bc7500d54390003e8000000000280";
    /// The temporal delimiter and sequence header that precede them in the file.
    const BEFORE: &str = "12000a0e02000005557ffc6af9d091009040";

    fn hex(bytes: &str) -> Vec<u8> {
        (0..bytes.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&bytes[at..at + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn a_real_encoder_volume_states_the_panel_that_a_decoder_reports() {
        // Every rational is what FFmpeg's own AV1 decoder exports for these
        // bytes, so this compares two readings of one file rather than one
        // reading of a hand-made payload.
        let hdr = hdr_from_packet(&hex(MDCV_OBU)).unwrap();
        let display = hdr.mastering.unwrap();
        let raw = |c: f64| (c * 65_536.0).round() as u32;
        assert_eq!((raw(display.red.x), raw(display.red.y)), (46_399, 19_137));
        assert_eq!(
            (raw(display.green.x), raw(display.green.y)),
            (11_141, 52_232)
        );
        assert_eq!((raw(display.blue.x), raw(display.blue.y)), (8_585, 3_015));
        assert_eq!(
            (raw(display.white.x), raw(display.white.y)),
            (20_493, 21_561)
        );
        assert_eq!((display.max_luminance * 256.0).round() as u32, 256_000);
        assert_eq!((display.min_luminance * 16_384.0).round() as u32, 2);
        assert!(display.is_hdr10());
        // Red carries the corner the command line listed last.
        assert!((display.red.x - 0.708).abs() < 1e-5);
        // A volume says nothing about the lights in the picture.
        assert_eq!(hdr.light, HdrMetadata::default().light);
    }

    #[test]
    fn a_real_encoder_light_level_survives_its_own_trailing_byte() {
        let hdr = hdr_from_packet(&hex(CLL_OBU)).unwrap();
        assert_eq!((hdr.light.max_cll, hdr.light.max_fall), (1234.0, 567.0));
        assert!(hdr.mastering.is_none());
    }

    #[test]
    fn a_real_first_frame_packet_yields_both_halves_of_its_metadata() {
        let mut packet = hex(BEFORE);
        packet.extend_from_slice(&hex(CLL_OBU));
        packet.extend_from_slice(&hex(MDCV_OBU));
        let obus = Obus::new(&packet).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(obus.len(), 4);
        assert_eq!((obus[0].kind, obus[1].kind), (2, 1));
        let hdr = hdr_from_packet(&packet).unwrap();
        assert!(hdr.mastering.unwrap().is_hdr10());
        assert_eq!(hdr.content_light(400.0).max_cll, 1234.0);
        assert_eq!(hdr.content_light(400.0).max_fall, 567.0);
    }

    #[test]
    fn a_content_light_payload_agrees_with_sei_144() {
        let packet = metadata_obu(METADATA_HDR_CLL, &[0x04, 0xd2, 0x02, 0x37]);
        let hdr = hdr_from_packet(&packet).unwrap();
        assert_eq!((hdr.light.max_cll, hdr.light.max_fall), (1234.0, 567.0));
        assert_eq!(
            hdr.light,
            HdrMetadata::from_clli(&[0x04, 0xd2, 0x02, 0x37])
                .unwrap()
                .light
        );
    }

    /// The volume payload of the SEI 137 unit x265 really wrote, so that one
    /// set of bytes can be read both ways.
    const SEI_MDCV_BYTES: &str = "21349baa199608fc8a4839083d1340420098968000000001";

    #[test]
    fn the_two_standards_do_not_share_a_volume_payload() {
        // The same 24 bytes, read with AV1's table: finite, in range, and a
        // different panel than the one H.265 means by them.
        let bytes = hex(SEI_MDCV_BYTES);
        let av1 = hdr_from_packet(&metadata_obu(METADATA_HDR_MDCV, &bytes)).unwrap();
        assert!(!av1.mastering.unwrap().is_hdr10());
        assert!((av1.mastering.unwrap().red.x - 0.1297).abs() < 0.001);
        assert!(HdrMetadata::from_mdcv(&bytes)
            .unwrap()
            .mastering
            .unwrap()
            .is_hdr10());
    }

    #[test]
    fn other_metadata_types_and_obus_of_other_kinds_state_nothing() {
        for kind in [3u64, 4, 5, 255] {
            assert!(hdr_from_packet(&metadata_obu(kind, &[0; MDCV_BODY_LEN]))
                .unwrap()
                .is_empty());
        }
        // A sequence header OBU is not read for metadata.
        let packet = vec![0x0a, 0x04, 0x1f, 0x00, 0x00, 0x00];
        assert_eq!(Obus::new(&packet).next().unwrap().unwrap().kind, 1);
        assert!(hdr_from_packet(&packet).unwrap().is_empty());
        // A frame OBU whose body would parse as a volume is still not metadata.
        let mut frame = vec![0x32, MDCV_BODY_LEN as u8];
        frame.extend_from_slice(&hdr10_body());
        assert!(hdr_from_packet(&frame).unwrap().is_empty());
    }

    #[test]
    fn a_type_may_be_written_over_several_bytes_and_a_truncated_one_is_not_a_volume() {
        let mut packet = vec![0x2a, 26, 0x82, 0x00]; // leb128 2, then the body
        packet.extend_from_slice(&hdr10_body());
        let hdr = hdr_from_packet(&packet).unwrap();
        assert!(hdr.mastering.unwrap().is_hdr10());
        // A body one byte short of the syntax states no volume.
        let short = metadata_obu(METADATA_HDR_MDCV, &hdr10_body()[..23]);
        assert!(hdr_from_packet(&short).unwrap().is_empty());
        let clli_short = metadata_obu(METADATA_HDR_CLL, &[0x04, 0xd2]);
        assert!(hdr_from_packet(&clli_short).unwrap().is_empty());
        // And a type whose LEB128 never ends is an error, not silence.
        assert!(metadata_type(&[0x80; 9]).is_err());
        assert!(metadata_type(&[]).is_err());
    }

    #[test]
    fn one_packet_can_carry_both_halves_and_the_volume_and_light_add_up() {
        let mut packet = metadata_obu(METADATA_HDR_MDCV, &hdr10_body());
        let light = metadata_obu(METADATA_HDR_CLL, &[0x04, 0xd2, 0x02, 0x37]);
        packet.extend_from_slice(&light);
        let hdr = hdr_from_packet(&packet).unwrap();
        assert!(hdr.mastering.unwrap().is_hdr10());
        assert_eq!((hdr.light.max_cll, hdr.light.max_fall), (1234.0, 567.0));
        assert_eq!(hdr.content_light(400.0).max_cll, 1234.0);
    }

    #[test]
    fn fields_beyond_the_payload_read_as_their_missing_high_order_bytes() {
        // Only ever reached through the length checks above, but the reader
        // must not index past a slice if a future caller loosens them.
        assert_eq!(field(&[0x01, 0x23], 0, 2), 0x0123);
        assert_eq!(field(&[0x01], 0, 2), 0);
        assert_eq!(field(&[0x01, 0x23, 0x45], 1, 2), 0x2345);
        assert_eq!(field(&[], 0, 4), 0);
    }
}
