//! HDR → display tone mapping in absolute-linear (cd/m²) light.
//!
//! Curves act on luma and rescale all three channels by the same factor, so
//! chroma survives below the knee instead of drifting per channel.

use crate::color::primaries::Primaries;

/// What the destination display can show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayTarget {
    /// Peak luminance in cd/m².
    pub peak_nits: f32,
    /// Diffuse white in cd/m²; content at or below this is mapped 1:1.
    pub paper_white_nits: f32,
    /// Minimum (black) luminance in cd/m².
    pub black_nits: f32,
}

impl Default for DisplayTarget {
    fn default() -> Self {
        Self::sdr(100.0)
    }
}

impl DisplayTarget {
    /// An SDR Rec.709 display: its diffuse white is its own peak.
    pub const fn sdr(peak_nits: f32) -> Self {
        Self {
            peak_nits,
            paper_white_nits: peak_nits,
            black_nits: 0.0,
        }
    }

    /// An HDR display: reference white moves up with panel capability.
    pub const fn hdr(peak_nits: f32) -> Self {
        Self {
            peak_nits,
            paper_white_nits: 262.0,
            black_nits: 0.0,
        }
    }

    /// Nominal range of the display, used to normalise curves.
    pub fn range(self) -> f32 {
        (self.peak_nits - self.black_nits).max(1.0)
    }
}

/// Static HDR content limits from the bitstream (HDR10 MDCV / CLLI).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContentLight {
    /// MaxCLL in cd/m², 0 when unknown.
    pub max_cll: f32,
    /// MaxFALL in cd/m², 0 when unknown.
    pub max_fall: f32,
}

impl ContentLight {
    /// A usable content peak: fall back to the display's own headroom.
    pub fn peak_or(self, fallback: f32) -> f32 {
        if self.max_cll > 0.0 {
            self.max_cll
        } else {
            fallback
        }
    }
}

/// Which tone-curve operator to run in the display domain.
///
/// The set and the parameter semantics mirror FFmpeg's `tonemap` filter so the
/// two can be compared sample for sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToneMap {
    /// Divide through by the content peak.
    Linear,
    /// Power-law shoulder above an adaptive break.
    Gamma,
    /// Hard clip in the display domain.
    #[default]
    Clip,
    /// Extended Reinhard.
    Reinhard,
    /// Hable/Uncharted 2 filmic curve.
    Hable,
    /// Möbius transform: linear below the joint, rational shoulder above.
    Mobius,
}

impl ToneMap {
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Gamma => "gamma",
            Self::Clip => "clip",
            Self::Reinhard => "reinhard",
            Self::Hable => "hable",
            Self::Mobius => "mobius",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Some(match label {
            "linear" => Self::Linear,
            "gamma" => Self::Gamma,
            "clip" => Self::Clip,
            "reinhard" => Self::Reinhard,
            "hable" => Self::Hable,
            "mobius" => Self::Mobius,
            _ => return None,
        })
    }

    pub const ALL: [ToneMap; 6] = [
        Self::Linear,
        Self::Gamma,
        Self::Clip,
        Self::Reinhard,
        Self::Hable,
        Self::Mobius,
    ];
}

/// Map one normalised linear value into `[0, 1]`, where 1.0 is the display's
/// peak and `peak` is the content's own peak in the same units.
///
/// `param` carries the per-curve knob, with FFmpeg's meaning: the scale for
/// [`ToneMap::Linear`] and [`ToneMap::Clip`], the exponent for
/// [`ToneMap::Gamma`], the reinforcement constant for [`ToneMap::Reinhard`]
/// and the joint below which [`ToneMap::Mobius`] stays 1:1.
pub fn curve(mode: ToneMap, x: f32, param: f32, peak: f32) -> f32 {
    let x = x.max(0.0);
    let peak = peak.max(1.0);
    match mode {
        ToneMap::Linear => x * param / peak,
        ToneMap::Gamma => {
            let e = 1.0 / param.max(0.01);
            if x > 0.05 {
                (x / peak).powf(e)
            } else {
                x * (0.05f32 / peak).powf(e) / 0.05
            }
        }
        ToneMap::Clip => (x * param).clamp(0.0, 1.0),
        ToneMap::Reinhard => {
            let p = param.max(1e-3);
            x / (x + p) * (peak + p) / peak
        }
        ToneMap::Hable => (hable(x) / hable(peak)).clamp(0.0, 1.0),
        ToneMap::Mobius => {
            let j = param.clamp(0.0, 0.999);
            if x <= j {
                return x;
            }
            let a = -j * j * (peak - 1.0) / (j * j - 2.0 * j + peak);
            let b = (j * j - 2.0 * j * peak + peak) / (peak - 1.0).max(1e-6);
            let scale = (b * b + 2.0 * b * j + j * j) / (b - a);
            (scale * (x + a) / (x + b)).clamp(0.0, 1.0)
        }
    }
}

/// Hable/Uncharted 2 filmic response, unnormalised.
fn hable(x: f32) -> f32 {
    let (a, b, c, d, e, f) = (0.15f32, 0.50, 0.10, 0.20, 0.02, 0.30);
    let num = x.mul_add(a * x + c * b, d * e);
    let den = x.mul_add(a * x + b, d * f);
    num / den - e / f
}

/// Tone map a linear-nits RGB triplet for one display.
///
/// Brightness is compressed as a single gain over the desaturated triplet, so
/// hue survives the shoulder; chroma is pulled toward luma only above the
/// display's paper white. Returns display-referred light where 1.0 is the
/// panel's peak.
pub fn tone_map_rgb(
    rgb_nits: [f32; 3],
    mode: ToneMap,
    target: DisplayTarget,
    content: ContentLight,
    primaries: Primaries,
) -> [f32; 3] {
    let range = target.range();
    let (kr, kb) = primaries.kr_kb();
    let (kr, kb) = (kr as f32, kb as f32);
    let unit = rgb_nits.map(|v| (v / range).max(0.0));
    let luma = luma_nits(unit, kr, kb);
    let desat = (target.paper_white_nits / range).max(1e-3);
    let overbright = (luma - desat).max(1e-6) / luma.max(1e-6);
    let mixed = unit.map(|v| v + (luma - v) * overbright);
    let peak = (content.peak_or(range) / range).max(1.0);
    let param = match mode {
        ToneMap::Linear | ToneMap::Clip => 1.0,
        ToneMap::Gamma => 0.5,
        ToneMap::Reinhard => 1.0,
        ToneMap::Hable => 0.0,
        ToneMap::Mobius => 0.3,
    };
    let sig = mixed.into_iter().fold(0.0f32, f32::max).max(1e-6);
    let gain = curve(mode, sig, param, peak) / sig;
    mixed.map(|v| (v * gain).clamp(0.0, 1.0))
}

/// Luma of a linear-nits triplet under the given primaries.
pub fn luma_nits(rgb_nits: [f32; 3], kr: f32, kb: f32) -> f32 {
    rgb_nits[0] * kr + rgb_nits[1] * (1.0 - kr - kb) + rgb_nits[2] * kb
}

/// Compress an out-of-gamut RGB triplet along its own saturation axis.
///
/// Input and output are normalised linear light in `primaries`. Only a negative
/// leg counts as out of gamut: in a relative-linear pipeline a leg above 1.0 is
/// super-white, and capping it here would discard exactly the highlights the
/// tone mapper downstream exists to fit. Luminance and hue survive; the chroma
/// is scaled by the least that brings every leg back to zero or above.
pub fn compress_gamut(rgb: [f32; 3], primaries: Primaries) -> [f32; 3] {
    if rgb.iter().all(|v| *v >= 0.0) {
        return rgb;
    }
    let (kr, kb) = primaries.kr_kb();
    let (kr, kb) = (kr as f32, kb as f32);
    let y = luma_nits(rgb, kr, kb);
    if y <= 0.0 {
        return [0.0; 3];
    }
    let mut scale = 1.0f32;
    for v in rgb {
        if v < 0.0 {
            scale = scale.min(y / (y - v));
        }
    }
    rgb.map(|v| (y + (v - y) * scale).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::primaries::rgb_to_rgb;
    use crate::color::{apply, Transfer};

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn every_curve_is_monotonic_and_stays_in_range() {
        for (mode, param) in [
            (ToneMap::Linear, 1.0),
            (ToneMap::Gamma, 0.5),
            (ToneMap::Clip, 1.0),
            (ToneMap::Reinhard, 1.0),
            (ToneMap::Hable, 0.0),
            (ToneMap::Mobius, 0.3),
        ] {
            let peak = 4.0;
            let mut prev = -1.0;
            for i in 0..=2000u32 {
                let x = i as f32 / 500.0;
                let y = curve(mode, x, param, peak);
                assert!(y >= -1e-6 && y <= 1.0 + 1e-6, "{mode:?} {x} -> {y}");
                assert!(
                    y + 1e-5 >= prev,
                    "{mode:?} not monotonic at {x}: {prev} -> {y}"
                );
                prev = y;
            }
            assert!(close(curve(mode, 0.0, param, peak), 0.0, 1e-6), "{mode:?}");
            assert!(close(curve(mode, peak, param, peak), 1.0, 1e-5), "{mode:?}");
        }
    }

    #[test]
    fn curves_match_the_ffmpeg_tonemap_reference_values() {
        // Transcribed from libavfilter/vf_tonemap.c at content peak 4.0, where
        // 1.0 is the display peak. Any slip in the port shows up here.
        let peak = 4.0;
        for (x, hable_out, mobius_out, reinhard_out, gamma_out) in [
            (0.05f32, 0.026_799_5, 0.05, 0.059_523_8, 0.000_156_3),
            (0.18, 0.094_091_8, 0.18, 0.190_678_0, 0.002_025_0),
            (0.5, 0.241_111_2, 0.462_382_4, 0.416_666_7, 0.015_625_0),
            (1.0, 0.426_646_6, 0.686_567_2, 0.625_000_0, 0.062_500_0),
            (2.0, 0.691_099_7, 0.872_561_8, 0.833_333_3, 0.250_000_0),
        ] {
            assert!(
                close(curve(ToneMap::Hable, x, 0.0, peak), hable_out, 1e-5),
                "hable {x}"
            );
            assert!(
                close(curve(ToneMap::Mobius, x, 0.3, peak), mobius_out, 1e-5),
                "mobius {x}"
            );
            assert!(
                close(curve(ToneMap::Reinhard, x, 1.0, peak), reinhard_out, 1e-5),
                "reinhard {x}"
            );
            assert!(
                close(curve(ToneMap::Gamma, x, 0.5, peak), gamma_out, 1e-5),
                "gamma {x}"
            );
        }
        // Möbius is 1:1 up to its joint whatever the peak.
        for peak in [1.0f32, 2.5, 10.0] {
            for x in [0.0f32, 0.1, 0.25, 0.3] {
                assert!(
                    close(curve(ToneMap::Mobius, x, 0.3, peak), x, 1e-6),
                    "{peak} {x}"
                );
            }
        }
    }

    #[test]
    fn clip_leaves_sdr_content_untouched() {
        let target = DisplayTarget::sdr(100.0);
        for x in [0.0f32, 0.2, 0.5, 1.0] {
            let out = tone_map_rgb(
                [x * 100.0, x * 100.0, x * 100.0],
                ToneMap::Clip,
                target,
                ContentLight::default(),
                Primaries::BT709,
            );
            assert!(close(out[0], x, 1e-6), "{x} -> {out:?}");
        }
    }

    #[test]
    fn tone_map_sends_the_content_peak_to_display_peak() {
        let target = DisplayTarget::sdr(100.0);
        let content = ContentLight {
            max_cll: 1000.0,
            max_fall: 400.0,
        };
        for mode in ToneMap::ALL {
            // A grey at the signalled content peak is the one input the curve
            // has to place exactly on the display peak; a coloured sample
            // arrives at the gain stage already narrowed by desaturation.
            let hot = tone_map_rgb([1000.0; 3], mode, target, content, Primaries::BT2020);
            assert!(close(hot[0], 1.0, 1e-4), "{mode:?} {hot:?}");
            let black = tone_map_rgb([0.0; 3], mode, target, content, Primaries::BT2020);
            assert!(close(black[0], 0.0, 1e-6), "{mode:?} {black:?}");
            // 20 nits, not 100: on an sdr(100) target diffuse white already
            // sits at the display peak, which the clip operator saturates.
            let mid = tone_map_rgb([20.0; 3], mode, target, content, Primaries::BT2020);
            assert!(mid[0] > 0.0 && mid[0] < hot[0], "{mode:?} {mid:?}");
        }
    }

    #[test]
    fn tone_map_preserves_hue_below_the_joint() {
        let target = DisplayTarget::sdr(203.0);
        let rgb = [80.0f32, 40.0, 20.0];
        for mode in [ToneMap::Clip, ToneMap::Reinhard, ToneMap::Hable] {
            let out = tone_map_rgb(rgb, mode, target, ContentLight::default(), Primaries::BT709);
            assert!(
                close(out[0] / out[1], rgb[0] / rgb[1], 1e-4),
                "{mode:?} {out:?}"
            );
            assert!(
                close(out[2] / out[1], rgb[2] / rgb[1], 1e-4),
                "{mode:?} {out:?}"
            );
        }
    }

    #[test]
    fn tone_map_is_channel_uniform_for_greys() {
        let target = DisplayTarget::hdr(1000.0);
        let content = ContentLight {
            max_cll: 4000.0,
            ..Default::default()
        };
        for mode in ToneMap::ALL {
            let out = tone_map_rgb(
                [500.0, 500.0, 500.0],
                mode,
                target,
                content,
                Primaries::BT2020,
            );
            assert!(
                close(out[0], out[1], 1e-6) && close(out[1], out[2], 1e-6),
                "{mode:?} {out:?}"
            );
        }
    }

    #[test]
    fn highlight_desaturation_kicks_in_above_paper_white() {
        let target = DisplayTarget::sdr(100.0);
        let content = ContentLight {
            max_cll: 400.0,
            ..Default::default()
        };
        let dark = tone_map_rgb(
            [80.0, 20.0, 20.0],
            ToneMap::Reinhard,
            target,
            content,
            Primaries::BT709,
        );
        let bright = tone_map_rgb(
            [320.0, 80.0, 80.0],
            ToneMap::Reinhard,
            target,
            content,
            Primaries::BT709,
        );
        // An over-bright sample is also a darker one after the gain, so
        // chroma only means something relative to the sample's own luma.
        let (kr, kb) = Primaries::BT709.kr_kb();
        let (kr, kb) = (kr as f32, kb as f32);
        let saturation = |v: [f32; 3]| {
            let y = luma_nits(v, kr, kb);
            (v[0] - v[1]) / y.max(1e-6)
        };
        let ratio = saturation(bright) / saturation(dark);
        assert!(
            ratio < 0.8,
            "chroma did not narrow: {dark:?} {bright:?} {ratio}"
        );
    }

    #[test]
    fn curve_labels_round_trip() {
        for mode in ToneMap::ALL {
            assert_eq!(ToneMap::from_label(mode.label()), Some(mode), "{mode:?}");
        }
        assert_eq!(ToneMap::from_label("nonsense"), None);
    }

    #[test]
    fn pq_code_maps_through_a_full_pipeline() {
        // A PQ code for 1000 cd/m² has to arrive brighter than diffuse white.
        let hi = Transfer::Pq.from_nits(1000.0, 203.0).unwrap();
        let hi_nits = Transfer::Pq.to_nits(hi, 203.0).unwrap();
        assert!(close(hi_nits, 1000.0, 0.5));
        let target = DisplayTarget::sdr(203.0);
        let content = ContentLight {
            max_cll: 1000.0,
            ..Default::default()
        };
        let mapped_hi = tone_map_rgb(
            [hi_nits; 3],
            ToneMap::Reinhard,
            target,
            content,
            Primaries::BT2020,
        );
        let mapped_lo = tone_map_rgb(
            [203.0; 3],
            ToneMap::Reinhard,
            target,
            content,
            Primaries::BT2020,
        );
        assert!(mapped_hi[0] > mapped_lo[0], "{mapped_hi:?} {mapped_lo:?}");
        assert!(mapped_hi[0] <= 1.0);
    }

    #[test]
    fn gamut_compression_fixes_negative_legs_and_keeps_super_white() {
        let inside = [0.2f32, 0.7, 0.1];
        assert_eq!(compress_gamut(inside, Primaries::BT709), inside);
        // A leg above 1.0 is a highlight, not a gamut error, so it survives.
        let bright = [1.4f32, 1.2, 0.5];
        assert_eq!(compress_gamut(bright, Primaries::BT709), bright);

        let rgb = [1.4f32, -0.3, 0.5];
        let out = compress_gamut(rgb, Primaries::BT709);
        assert!(out.iter().all(|v| *v >= 0.0), "{out:?}");
        assert!(out[1] < out[0], "{out:?}");
        // The fix is a pure chroma scale, so luma and hue both carry over.
        let (kr, kb) = Primaries::BT709.kr_kb();
        let (kr, kb) = (kr as f32, kb as f32);
        assert!(close(luma_nits(out, kr, kb), luma_nits(rgb, kr, kb), 1e-5));
        let ratio = |v: [f32; 3]| (v[0] - v[1]) / (v[2] - v[1]);
        assert!(close(ratio(out), ratio(rgb), 1e-4), "{out:?}");
    }

    #[test]
    fn bt2020_to_bt709_uses_an_accurate_matrix() {
        let m = rgb_to_rgb(Primaries::BT2020, Primaries::BT709);
        // BT.709 red sits inside BT.2020, so there and back must be exact.
        let red = apply(m, [1.0, 0.0, 0.0]);
        let back = apply(rgb_to_rgb(Primaries::BT709, Primaries::BT2020), red);
        assert!(back[0] > 0.999 && back[0] < 1.001, "{back:?}");
        // BT.2020 green and blue are outside BT.709 and need a negative leg.
        let green = apply(m, [0.0, 1.0, 0.0]);
        let blue = apply(m, [0.0, 0.0, 1.0]);
        assert!(green[0] < 0.0 && green[2] < 0.0, "{green:?}");
        assert!(blue[0] < 0.0 && blue[1] < 0.0, "{blue:?}");
    }
}
