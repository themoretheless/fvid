//! Static HDR signalling: the mastering display a grade was shot on and the
//! light level the content actually reaches.
//!
//! Both arrive as byte-aligned integer payloads rather than bit fields, so a
//! payload can be decoded without the bitstream reader that surrounds it. The
//! framing differs per container — an HEVC SEI message, an MP4 `mdcv`/`ccll`
//! box, a Matroska `MasteringMetadata` element — but the payload itself is the
//! same bytes in all three, which is why the decoders live here and not in
//! `crate::codec`.
//!
//! Units are the ones SMPTE ST 2086 prescribes: chromaticity coordinates in
//! multiples of 0.00002 and luminance in multiples of 0.0001 cd/m². The three
//! primaries are carried in **green, blue, red** order, which is the order
//! ITU-T H.265 indexes them; reading them as red, green, blue swaps two
//! corners of the mastering volume and silently shifts every saturated colour.

use crate::color::primaries::{Chromaticity, Primaries};
use crate::color::tonemap::{ContentLight, DisplayTarget};

/// One chromaticity unit, in CIE 1931 coordinate space.
const CHROMA_STEP: f64 = 0.00002;
/// One luminance unit, in cd/m².
const LUMA_STEP: f32 = 0.0001;

/// Payload length of a mastering display colour volume block: three primaries
/// and a white point as x/y pairs, then peak and minimum luminance.
pub const MDCV_PAYLOAD_LEN: usize = 24;
/// Payload length of a content light level block: MaxCLL then MaxFALL.
pub const CLLI_PAYLOAD_LEN: usize = 4;

/// SEI payload type carrying the mastering display, ITU-T H.265 D.2.28.
pub const SEI_MDCV: u8 = 137;
/// SEI payload type carrying the content light level, ITU-T H.265 D.2.29.
pub const SEI_CLLI: u8 = 144;

fn chroma(raw: u16) -> f64 {
    f64::from(raw) * CHROMA_STEP
}

fn luminance(raw: u32) -> f32 {
    raw as f32 * LUMA_STEP
}

/// The display a master was graded for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasteringDisplay {
    pub red: Chromaticity,
    pub green: Chromaticity,
    pub blue: Chromaticity,
    pub white: Chromaticity,
    /// Peak white in cd/m².
    pub max_luminance: f32,
    /// Darkest displayable black in cd/m².
    pub min_luminance: f32,
}

impl MasteringDisplay {
    /// Decode a mastering display colour volume payload.
    ///
    /// The coordinates are read in the standard's green, blue, red order and
    /// placed into the corners they name.
    pub fn from_payload(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < MDCV_PAYLOAD_LEN {
            return None;
        }
        let mut pairs = [(0u16, 0u16); 4];
        for (slot, pair) in pairs.iter_mut().enumerate() {
            *pair = (
                be16(&bytes[slot * 4..slot * 4 + 2]),
                be16(&bytes[slot * 4 + 2..slot * 4 + 4]),
            );
        }
        let point = |(x, y): (u16, u16)| Chromaticity {
            x: chroma(x),
            y: chroma(y),
        };
        Some(Self {
            green: point(pairs[0]),
            blue: point(pairs[1]),
            red: point(pairs[2]),
            white: point(pairs[3]),
            max_luminance: luminance(be32(&bytes[16..20])),
            min_luminance: luminance(be32(&bytes[20..24])),
        })
    }

    /// The primaries of this display, usable directly as a conversion source or
    /// destination.
    pub fn primaries(&self) -> Primaries {
        Primaries {
            r: self.red,
            g: self.green,
            b: self.blue,
            white: self.white,
        }
    }

    /// This display as a tone-map destination: what it can show is what the
    /// highlights have to fit into.
    pub fn display_target(&self) -> DisplayTarget {
        let mut target = DisplayTarget::hdr(self.max_luminance);
        target.black_nits = self.min_luminance;
        target
    }

    /// True when the volume is the HDR10 master display: BT.2020 corners on a
    /// D65 white, 1 000 cd/m² peak.
    ///
    /// The tolerance covers the coordinate rounding a tool chain introduces;
    /// a display that fails this is a genuinely different panel, such as a
    /// wide-gamut projector or the 4 000 cd/m² profile of HDR10+.
    pub fn is_hdr10(&self) -> bool {
        let close = |a: f64, b: f64| (a - b).abs() <= 0.002;
        let eq = |a: Chromaticity, b: Chromaticity| close(a.x, b.x) && close(a.y, b.y);
        eq(self.red, Primaries::BT2020.r)
            && eq(self.green, Primaries::BT2020.g)
            && eq(self.blue, Primaries::BT2020.b)
            && eq(self.white, Primaries::BT2020.white)
            && (self.max_luminance - 1_000.0).abs() <= 0.5
    }
}

/// Everything a decoder needs to tone map without being told.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HdrMetadata {
    pub mastering: Option<MasteringDisplay>,
    pub light: ContentLight,
}

impl HdrMetadata {
    /// Metadata from a content light level payload.
    pub fn from_clli(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < CLLI_PAYLOAD_LEN {
            return None;
        }
        Some(Self {
            mastering: None,
            light: ContentLight {
                max_cll: f32::from(be16(&bytes[..2])),
                max_fall: f32::from(be16(&bytes[2..])),
            },
        })
    }

    /// Metadata from a mastering display payload.
    pub fn from_mdcv(bytes: &[u8]) -> Option<Self> {
        Some(Self {
            mastering: MasteringDisplay::from_payload(bytes),
            light: ContentLight::default(),
        })
    }

    /// Take whichever fields the newer block actually carries.
    ///
    /// A stream can signal the two halves in different places — an MP4 `mdcv`
    /// box next to an in-band SEI 144, for instance — so a block that only
    /// knows one of them must not clear the other.
    pub fn merge(&mut self, other: Self) {
        if other.mastering.is_some() {
            self.mastering = other.mastering;
        }
        if other.light.max_cll > 0.0 || other.light.max_fall > 0.0 {
            self.light = other.light;
        }
    }

    /// Content peak to tone map from, given a fallback for streams that carry
    /// no light level at all.
    pub fn content_light(&self, fallback_peak: f32) -> ContentLight {
        ContentLight {
            max_cll: self.light.peak_or(fallback_peak),
            max_fall: self.light.max_fall,
        }
    }

    /// True when nothing was signalled.
    pub fn is_empty(&self) -> bool {
        self.mastering.is_none() && self.light.max_cll == 0.0 && self.light.max_fall == 0.0
    }
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Encode a mastering display payload, the exact inverse of
/// [`MasteringDisplay::from_payload`].
///
/// Writers need this as much as readers need the decoder: an HDR10 file cannot
/// be stamped correctly without knowing that the corners go out green, blue,
/// red.
pub fn mdcv_payload(display: &MasteringDisplay) -> [u8; MDCV_PAYLOAD_LEN] {
    let q = |c: Chromaticity| {
        (
            (c.x / CHROMA_STEP).round() as u16,
            (c.y / CHROMA_STEP).round() as u16,
        )
    };
    let l = |n: f32| (n / LUMA_STEP).round() as u32;
    let (gx, gy) = q(display.green);
    let (bx, by) = q(display.blue);
    let (rx, ry) = q(display.red);
    let (wx, wy) = q(display.white);
    let mut out = [0u8; MDCV_PAYLOAD_LEN];
    for (i, v) in [gx, gy, bx, by, rx, ry, wx, wy].iter().enumerate() {
        out[i * 2..i * 2 + 2].copy_from_slice(&v.to_be_bytes());
    }
    out[16..20].copy_from_slice(&l(display.max_luminance).to_be_bytes());
    out[20..24].copy_from_slice(&l(display.min_luminance).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    /// BT.2020 corners and a 1 000 cd/m² peak, in ST 2086 integer units, in the
    /// order the payload carries them: green, blue, red, white.
    fn hdr10_bytes() -> Vec<u8> {
        let mut v = Vec::new();
        for pair in [
            [8_500u16, 39_850],
            [6_550, 2_300],
            [35_400, 14_600],
            [15_635, 16_450],
        ] {
            for x in pair {
                v.extend_from_slice(&x.to_be_bytes());
            }
        }
        v.extend_from_slice(&10_000_000u32.to_be_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        v
    }

    #[test]
    fn mdcv_reads_the_green_blue_red_order_the_standard_writes() {
        let d = MasteringDisplay::from_payload(&hdr10_bytes()).unwrap();
        // Each corner is a different value on every axis, so a wrong order
        // cannot leave any of these comparisons standing.
        assert!(
            close(d.green.x, 0.170, 1e-6) && close(d.green.y, 0.797, 1e-6),
            "{d:?}"
        );
        assert!(
            close(d.blue.x, 0.131, 1e-6) && close(d.blue.y, 0.046, 1e-6),
            "{d:?}"
        );
        assert!(
            close(d.red.x, 0.708, 1e-6) && close(d.red.y, 0.292, 1e-6),
            "{d:?}"
        );
        assert!(close(d.white.x, 0.3127, 1e-4), "{:?}", d.white);
        assert_eq!(d.max_luminance, 1_000.0);
        assert_eq!(d.min_luminance, 0.0001);
        assert!(d.is_hdr10());
    }

    #[test]
    fn a_read_garbage_order_volume_is_not_hdr10() {
        // The same bytes read as red, green, blue — the mistake this payload's
        // documentation exists to prevent.
        let bytes = hdr10_bytes();
        // The same BT.2020 corners written in the intuitive red, green, blue
        // order instead of the standard's green, blue, red.
        let swapped = {
            let mut b = bytes.clone();
            b[..4].copy_from_slice(&bytes[8..12]);
            b[4..8].copy_from_slice(&bytes[..4]);
            b[8..12].copy_from_slice(&bytes[4..8]);
            b
        };
        let d = MasteringDisplay::from_payload(&swapped).unwrap();
        assert!(!d.is_hdr10());
        assert!(
            close(d.red.x, 0.131, 1e-6) && close(d.red.y, 0.046, 1e-6),
            "{:?}",
            d.red
        );
    }

    #[test]
    fn mdcv_payload_round_trips() {
        let d = MasteringDisplay::from_payload(&hdr10_bytes()).unwrap();
        let back = MasteringDisplay::from_payload(&mdcv_payload(&d)).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn mastering_display_feeds_the_tone_mapper_as_a_panel() {
        let d = MasteringDisplay::from_payload(&hdr10_bytes()).unwrap();
        let target = d.display_target();
        assert_eq!(target.peak_nits, 1_000.0);
        assert_eq!(target.black_nits, 0.0001);
        // Reference white stays at BT.2100's 262 cd/m² for a panel this bright.
        assert_eq!(target.paper_white_nits, 262.0);
        // The primaries are usable as a conversion source, which is how a
        // non-BT.2020 master gets mapped.
        let p = d.primaries();
        assert!(close(p.kr_kb().0, 0.2627, 5e-4), "{p:?}");
    }

    #[test]
    fn clli_reads_maxcll_and_maxfall_in_candelas() {
        // The four bytes an x265 max-cll=1234,567 encode produces.
        let m = HdrMetadata::from_clli(&[0x04, 0xD2, 0x02, 0x37]).unwrap();
        assert_eq!(m.light.max_cll, 1_234.0);
        assert_eq!(m.light.max_fall, 567.0);
        assert!(m.mastering.is_none());
        assert_eq!(m.content_light(1_000.0).max_cll, 1_234.0);
    }

    #[test]
    fn empty_light_falls_back_to_the_assumed_peak() {
        let mut m = HdrMetadata::from_clli(&[0, 0, 0, 0]).unwrap();
        assert!(m.is_empty());
        assert_eq!(m.content_light(4_000.0).max_cll, 4_000.0);
        m.merge(HdrMetadata::from_mdcv(&hdr10_bytes()).unwrap());
        assert!(!m.is_empty());
        assert!(m.mastering.is_some());
        // A mastering block alone must not erase light level it already had...
        m.merge(HdrMetadata::from_clli(&[0x0F, 0xA0, 0x01, 0x90]).unwrap());
        assert!(m.mastering.is_some());
        assert_eq!(m.light.max_cll, 4_000.0);
    }

    #[test]
    fn short_payloads_are_refused_not_panicked() {
        for len in 0..MDCV_PAYLOAD_LEN {
            assert!(
                MasteringDisplay::from_payload(&hdr10_bytes()[..len]).is_none(),
                "{len}"
            );
        }
        for len in 0..CLLI_PAYLOAD_LEN {
            assert!(
                HdrMetadata::from_clli(&[0x04u8; CLLI_PAYLOAD_LEN][..len]).is_none(),
                "{len}"
            );
        }
        // Extra trailing bytes are fine: a payload can be the start of a
        // larger SEI message with its trailing alignment bits still attached.
        let mut long = hdr10_bytes();
        long.push(0x80);
        assert!(MasteringDisplay::from_payload(&long).is_some());
    }
}
