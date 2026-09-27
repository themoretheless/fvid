//! Static HDR signalling: the mastering display a grade was shot on and the
//! light level the content actually reaches.
//!
//! The mastering display and light level travel as one byte-aligned payload
//! wherever they are stored as raw bytes: an HEVC SEI message and an MP4
//! `mdcv`/`ccll` box carry the same bytes, which is why those decoders live
//! here and not in `crate::codec`. Matroska spells the same numbers as one
//! element per value, so its reader builds through
//! [`MasteringDisplay::from_corners`] instead of a payload.
//!
//! Units of the payload are the ones SMPTE ST 2086 prescribes: chromaticity
//! coordinates in multiples of 0.00002 and luminance in multiples of 0.0001
//! cd/m². The three primaries are carried in **green, blue, red** order, which
//! is the order ITU-T H.265 indexes them; reading them as red, green, blue
//! swaps two corners of the mastering volume and silently shifts every
//! saturated colour.

use crate::color::primaries::{Chromaticity, MatrixCoeff, Primaries, YuvMatrix};
use crate::color::tonemap::{ContentLight, DisplayTarget};
use crate::color::transfer::Transfer;

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

fn chroma(raw: u32) -> f64 {
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
    /// Assemble a volume from coordinates and luminances already in their own
    /// units.
    ///
    /// This is the shape Matroska's `MasteringMetadata` and any in-memory
    /// source use, and the one place a volume can be rejected for describing
    /// no display: a chromaticity outside the unit square, or a negative
    /// luminance, is a file that has lost its own meaning.
    pub fn from_corners(
        red: (f64, f64),
        green: (f64, f64),
        blue: (f64, f64),
        white: (f64, f64),
        max_luminance: f32,
        min_luminance: f32,
    ) -> Option<Self> {
        let point = |(x, y): (f64, f64)| {
            (x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y))
                .then_some(Chromaticity { x, y })
        };
        let display = Some(Self {
            red: point(red)?,
            green: point(green)?,
            blue: point(blue)?,
            white: point(white)?,
            max_luminance,
            min_luminance,
        })?;
        (display.max_luminance.is_finite()
            && display.min_luminance.is_finite()
            && display.max_luminance >= 0.0
            && display.min_luminance >= 0.0)
            .then_some(display)
    }

    /// Decode a mastering display colour volume payload.
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
        let point = |p: (u16, u16)| (chroma(u32::from(p.0)), chroma(u32::from(p.1)));
        // The corners are carried in the standard's green, blue, red order and
        // placed into the corners they name.
        Self::from_corners(
            point(pairs[2]),
            point(pairs[0]),
            point(pairs[1]),
            point(pairs[3]),
            luminance(be32(&bytes[16..20])),
            luminance(be32(&bytes[20..24])),
        )
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

    /// This statement with each half it leaves unnamed taken from a weaker one,
    /// which in practice is a file's `mdcv`/`ccll` box or `Colour` element
    /// completed by the coding's own SEI message or metadata OBU. A half this
    /// statement names is kept whichever of the two it came from.
    pub fn filled_with(self, fallback: Self) -> Self {
        let unnamed = |light: &ContentLight| light.max_cll == 0.0 && light.max_fall == 0.0;
        Self {
            mastering: self.mastering.or(fallback.mastering),
            light: if unnamed(&self.light) {
                fallback.light
            } else {
                self.light
            },
        }
    }

    /// Content peak to tone map from, given a fallback for a stream that states
    /// no light at all.
    ///
    /// MaxCLL is the statement about the content, so it wins wherever it is
    /// written. A mastering volume names the peak the content was authored
    /// against, which answers for the many files that write `mdcv` and no
    /// `ccll`; the two are not maxed together, because a dim master under a
    /// bright display still deserves its highlights kept. Only when neither is
    /// stated does the caller's own headroom stand in.
    pub fn content_light(&self, fallback_peak: f32) -> ContentLight {
        let authored = self
            .mastering
            .map_or(fallback_peak, |volume| volume.max_luminance);
        ContentLight {
            max_cll: self.light.peak_or(authored),
            max_fall: self.light.max_fall,
        }
    }

    /// True when nothing was signalled.
    pub fn is_empty(&self) -> bool {
        self.mastering.is_none() && self.light.max_cll == 0.0 && self.light.max_fall == 0.0
    }
}

/// The ITU-T H.273 signal a container or parameter set states for a picture:
/// which primaries, which transfer curve, which luma matrix, and whether the
/// coded range is full.
///
/// This is the same four facts an MP4 `colr` atom, a Matroska `Colour` element
/// and an HEVC VUI each spell in their own syntax, so they are held once and
/// resolved through the code tables in `color`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColourDescription {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    /// Set when the coded range is full (0..max) rather than studio.
    pub full_range: bool,
}

impl ColourDescription {
    /// The named primary set, when the code means one this module can convert.
    pub fn primary_set(self) -> Option<Primaries> {
        Primaries::from_code(self.primaries)
    }

    /// The named transfer curve. Codes this module has no implementation for
    /// come back as [`Transfer::Unknown`] rather than a guess.
    pub fn transfer_function(self) -> Transfer {
        Transfer::from_code(self.transfer)
    }

    pub fn matrix_coefficients(self) -> Option<MatrixCoeff> {
        MatrixCoeff::from_code(self.matrix)
    }

    /// The RGB → YCbCr matrix this description calls for, when its luma weights
    /// are known.
    pub fn yuv_matrix(self) -> Option<YuvMatrix> {
        YuvMatrix::from_matrix(self.matrix_coefficients()?, self.full_range)
    }

    /// True when the stated curve is one of BT.2100's two, which is what makes
    /// a picture HDR whatever its container says.
    pub fn is_hdr(self) -> bool {
        self.transfer_function().is_hdr()
    }

    /// Describe a picture already decoded into these primaries and curve,
    /// keeping whatever range and matrix were stated.
    pub fn with_signal(self, primaries: Primaries, transfer: Transfer) -> Self {
        Self {
            primaries: primaries.code().unwrap_or(self.primaries),
            transfer: transfer.code(),
            ..self
        }
    }

    /// Fill each of the three codes this statement leaves unspecified from
    /// another statement of the same picture, which in practice is a file's
    /// `colr` atom or `Colour` element completed by the coding's own VUI or
    /// sequence header.
    ///
    /// H.273 numbers 0 as "unspecified", so 0 is the only value the fall-back
    /// may replace: a source that names a code keeps it whichever of the two it
    /// came from. A range flag has no unspecified value to be replaced by, so
    /// it falls back only when this statement named none of the three codes,
    /// which is the case of a container with no colour description at all.
    pub fn filled_with(self, fallback: Self) -> Self {
        let named = |mine: u8, theirs: u8| if mine == 0 { theirs } else { mine };
        let stated_triple = (self.primaries | self.transfer | self.matrix) != 0;
        Self {
            primaries: named(self.primaries, fallback.primaries),
            transfer: named(self.transfer, fallback.transfer),
            matrix: named(self.matrix, fallback.matrix),
            full_range: self.full_range || (!stated_triple && fallback.full_range),
        }
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

    /// The state a master that writes `mdcv` and no `ccll` leaves a reader in:
    /// the volume names the peak the content was authored against, and nothing
    /// states what the content itself reaches.
    #[test]
    fn a_mastering_volume_answers_for_a_peak_the_content_never_stated() {
        let mut m = HdrMetadata::from_mdcv(&hdr10_bytes()).unwrap();
        assert_eq!(m.light.max_cll, 0.0);
        // The authored 1 000 cd/m², not the 100-nit panel a grade is asked for.
        assert_eq!(m.content_light(100.0).max_cll, 1_000.0);
        // A dim master under a bright caller stays dim: the caller's headroom is
        // a fallback, not something to max the authored volume against.
        let volume = m.mastering.unwrap();
        m.mastering = Some(MasteringDisplay {
            max_luminance: 400.0,
            ..volume
        });
        assert_eq!(m.content_light(1_000.0).max_cll, 400.0);
        // Where the content does state its own peak, that statement wins.
        m.merge(HdrMetadata::from_clli(&[0x04, 0xD2, 0, 0]).unwrap());
        assert_eq!(m.content_light(100.0).max_cll, 1_234.0);
        assert!(m.mastering.is_some());
    }

    /// The direction a reader needs, which is the reverse of `merge`: a
    /// container's box owns whichever half it named, and the coding's in-band
    /// message counts only for the half the box never wrote.
    #[test]
    fn a_containers_box_owns_the_half_it_names() {
        let container = HdrMetadata::from_clli(&[0x04, 0xD2, 0x02, 0x37]).unwrap();
        let mut bitstream = HdrMetadata::from_mdcv(&hdr10_bytes()).unwrap();
        bitstream.merge(HdrMetadata::from_clli(&[0x03, 0xE8, 0x01, 0x90]).unwrap());
        let filled = container.filled_with(bitstream);
        assert!(filled.mastering.unwrap().is_hdr10());
        assert_eq!(
            (filled.light.max_cll, filled.light.max_fall),
            (1_234.0, 567.0)
        );
        // A box that states nothing takes both halves from the stream.
        assert_eq!(HdrMetadata::default().filled_with(bitstream), bitstream);
        // A box that states the volume keeps it against the stream's.
        let mut bright = HdrMetadata::from_clli(&[0x0F, 0xA0, 0x00, 0x00]).unwrap();
        bright.mastering = MasteringDisplay::from_corners(
            (0.7, 0.3),
            (0.2, 0.8),
            (0.13, 0.05),
            (0.3127, 0.329),
            4_000.0,
            0.0001,
        );
        let filled = bright.filled_with(bitstream);
        assert_eq!(filled.mastering, bright.mastering);
        assert_eq!(filled.light.max_cll, 4_000.0);
    }

    /// The same volume spelled the way Matroska spells it: a value per corner,
    /// already in its own units, in named order rather than the payload's.
    #[test]
    fn corners_and_payload_describe_the_same_volume() {
        let from_payload = MasteringDisplay::from_payload(&hdr10_bytes()).unwrap();
        let from_corners = MasteringDisplay::from_corners(
            (0.708, 0.292),
            (0.17, 0.797),
            (0.131, 0.046),
            (0.3127, 0.3290),
            1_000.0,
            0.0001,
        )
        .unwrap();
        assert!(from_corners.is_hdr10());
        // The payload quantises to ST 2086 units, so the two agree to that
        // step rather than bit for bit.
        assert_eq!(mdcv_payload(&from_corners), mdcv_payload(&from_payload));
    }

    /// A volume that names no corner in the unit square is not a display, and
    /// a tone map built from one would be worse than no tone map at all.
    #[test]
    fn a_volume_outside_the_unit_square_is_refused() {
        let red_at = |x: f64| {
            MasteringDisplay::from_corners(
                (x, 0.292),
                (0.17, 0.797),
                (0.131, 0.046),
                (0.3127, 0.329),
                1_000.0,
                0.0001,
            )
        };
        assert!(red_at(0.708).is_some());
        assert!(red_at(1.4).is_none());
        assert!(red_at(-0.1).is_none());
        assert!(red_at(f64::NAN).is_none());
        // A payload of all 0xff bytes puts 1.31 in every coordinate.
        assert!(MasteringDisplay::from_payload(&[0xff; MDCV_PAYLOAD_LEN]).is_none());
    }

    #[test]
    fn colour_description_resolves_h273_codes() {
        // BT.2020 primaries on PQ with the BT.2020 non-constant-luminance
        // matrix — the HDR10 signalling, code 9/16/9.
        let hdr10 = ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        };
        assert_eq!(hdr10.primary_set(), Some(Primaries::BT2020));
        assert_eq!(hdr10.transfer_function(), Transfer::Pq);
        assert!(hdr10.is_hdr());
        assert!(hdr10.yuv_matrix().is_some());
        // An SDR file's 1/1/1 is not HDR, and full range changes the matrix.
        let sdr = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        assert!(!sdr.is_hdr());
        assert_eq!(
            sdr.yuv_matrix(),
            YuvMatrix::new(0.2126, 0.0722, false).into()
        );
        assert_ne!(
            sdr.yuv_matrix(),
            ColourDescription {
                full_range: true,
                ..sdr
            }
            .yuv_matrix()
        );
        // Code 2 is "unspecified": the corner set is unknown but the curve of
        // a known transfer code still resolves.
        let vague = ColourDescription {
            primaries: 2,
            transfer: 13,
            matrix: 2,
            full_range: false,
        };
        assert_eq!(vague.primary_set(), None);
        assert_eq!(vague.matrix_coefficients(), None);
        assert_eq!(vague.transfer_function(), Transfer::Srgb);
        assert!(!vague.is_hdr());
        // Stating a decoded picture's own space keeps the range and matrix.
        let moved = sdr.with_signal(Primaries::BT2020, Transfer::Hlg);
        assert_eq!(
            moved,
            ColourDescription {
                primaries: 9,
                transfer: 18,
                ..sdr
            }
        );
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

    /// A container that names none of the three codes names no range beside
    /// them either, so a wholly silent triple is the one case where the
    /// coding's range flag carries over; a container that names any code owns
    /// the range it wrote with it.
    #[test]
    fn a_statement_fills_only_the_codes_it_leaves_unspecified() {
        let container = ColourDescription {
            primaries: 9,
            ..Default::default()
        };
        let bitstream = ColourDescription {
            primaries: 1,
            transfer: 16,
            matrix: 9,
            full_range: true,
        };
        assert_eq!(
            container.filled_with(bitstream),
            ColourDescription {
                primaries: 9,
                transfer: 16,
                matrix: 9,
                full_range: false,
            }
        );
        assert_eq!(
            ColourDescription::default().filled_with(bitstream),
            bitstream
        );
        assert!(
            ColourDescription {
                transfer: 1,
                full_range: true,
                ..Default::default()
            }
            .filled_with(ColourDescription::default())
            .full_range
        );
    }
}
