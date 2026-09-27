//! Signal ↔ light transfer functions: SDR gamma curves, PQ (ST 2084) and HLG.

/// Relative-linear luminance of SDR reference white, in cd/m².
pub const SDR_PEAK_NITS: f32 = 203.0;
/// PQ encodes an absolute luminance range of 0..10 000 cd/m².
pub const PQ_PEAK_NITS: f32 = 10_000.0;
/// HLG's inverse OETF yields scene-light peaks of 1 000 cd/m² before the OOTF.
pub const HLG_PEAK_NITS: f32 = 1_000.0;

/// ST 2084's own constants. The curves below evaluate in `f64` and round once:
/// `1/m1` is 6.28, so an `f32` intermediate would widen the error past the
/// published code values, which are quoted to seven decimals.
const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 32.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = (2413.0 / 4096.0) * 32.0;
const PQ_C3: f64 = (2392.0 / 4096.0) * 32.0;

const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 0.284_668_92;
const HLG_C: f32 = 0.559_910_73;

/// SMPTE 240M-1995 clause 4.3/4.4, in the first-derivative-continuous form
/// libzimg uses: the published 0.0228 breakpoint leaves a 6e-5 step.
const SMPTE240_ALPHA: f64 = 1.111_572_195_921_731;
const SMPTE240_BREAK_LIGHT: f64 = 0.022_821_585_529_445;
const SMPTE240_BREAK_SIGNAL: f64 = 0.091_286_342_117_78;

/// A transfer characteristic, named by its ITU-T H.273 signalling code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transfer {
    /// ITU-R BT.709, also used as the BT.601 and BT.2020 camera curve.
    Bt709,
    /// ITU-R BT.601-6 (identical piecewise curve to [`Transfer::Bt709`]).
    Bt601,
    /// SMPTE 240M.
    Bt240,
    /// ITU-R BT.470 System M, pure gamma 2.2.
    Gamma22,
    /// ITU-R BT.470 System B/G, pure gamma 2.8.
    Gamma28,
    /// IEC 61966-2-1 (sRGB / sYCC).
    Srgb,
    /// Linear light.
    Linear,
    /// IEC 61966-2-4 xvYCC.
    Xycc,
    /// SMPTE ST 2084 (PQ), absolute luminance up to 10 000 cd/m².
    Pq,
    /// ARIB STD-B67 (Hybrid Log-Gamma), scene-light.
    Hlg,
    /// Signalled but not decodable here.
    Unknown,
}

impl Transfer {
    /// Map an H.273 / AV1 `transfer_characteristics` code.
    pub fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Bt709,
            4 => Self::Gamma22,
            5 => Self::Gamma28,
            6 => Self::Bt601,
            7 => Self::Bt240,
            8 => Self::Linear,
            11 => Self::Xycc,
            12 => Self::Bt709,
            13 => Self::Srgb,
            14 | 15 => Self::Bt709,
            16 => Self::Pq,
            18 => Self::Hlg,
            _ => Self::Unknown,
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Bt709 => 1,
            Self::Gamma22 => 4,
            Self::Gamma28 => 5,
            Self::Bt601 => 6,
            Self::Bt240 => 7,
            Self::Linear => 8,
            Self::Xycc => 11,
            Self::Srgb => 13,
            Self::Pq => 16,
            Self::Hlg => 18,
            Self::Unknown => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bt709 => "BT.709",
            Self::Bt601 => "BT.601",
            Self::Bt240 => "SMPTE 240M",
            Self::Gamma22 => "gamma 2.2",
            Self::Gamma28 => "gamma 2.8",
            Self::Srgb => "sRGB",
            Self::Linear => "linear",
            Self::Xycc => "xvYCC",
            Self::Pq => "PQ",
            Self::Hlg => "HLG",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_hdr(self) -> bool {
        matches!(self, Self::Pq | Self::Hlg)
    }

    /// Signal (0..=1) to relative linear light: 1.0 is reference white for SDR
    /// and 10 000 cd/m² for PQ.
    pub fn eotf(self, signal: f32) -> Option<f32> {
        let v = signal.clamp(0.0, 1.0) as f64;
        let out = match self {
            Self::Linear | Self::Unknown => return None,
            Self::Pq => {
                let vm = v.powf(1.0 / PQ_M2);
                let num = (vm - PQ_C1).max(0.0);
                let den = (PQ_C2 - PQ_C3 * vm).max(f64::EPSILON);
                (num / den).powf(1.0 / PQ_M1)
            }
            Self::Hlg => return Some(hlg_inverse_oetf(signal)),
            Self::Gamma22 => v.powf(2.2),
            Self::Gamma28 => v.powf(2.8),
            Self::Srgb => {
                if v <= 0.040_45 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            }
            Self::Xycc => {
                if signal < 0.0 {
                    -1.03 * (signal.abs() as f64).powf(2.4)
                } else {
                    1.02 * v.powf(2.4) - 0.02
                }
            }
            Self::Bt240 => {
                if v < SMPTE240_BREAK_SIGNAL {
                    v / 4.0
                } else {
                    ((v + SMPTE240_ALPHA - 1.0) / SMPTE240_ALPHA).powf(1.0 / 0.45)
                }
            }
            Self::Bt709 | Self::Bt601 => {
                if v < 0.081 {
                    v / 4.5
                } else {
                    ((v + 0.099) / 1.099).powf(1.0 / 0.45)
                }
            }
        };
        Some(out as f32)
    }

    /// Relative linear light back to signal. Values above white are allowed so
    /// that highlights survive a round trip.
    pub fn oetf(self, linear: f32) -> Option<f32> {
        let l = linear as f64;
        let out = match self {
            Self::Linear | Self::Unknown => return None,
            Self::Pq => {
                let ym = l.max(0.0).powf(PQ_M1);
                ((PQ_C1 + PQ_C2 * ym) / (1.0 + PQ_C3 * ym)).powf(PQ_M2)
            }
            Self::Hlg => return Some(hlg_oetf(linear)),
            Self::Gamma22 => l.max(0.0).powf(1.0 / 2.2),
            Self::Gamma28 => l.max(0.0).powf(1.0 / 2.8),
            Self::Srgb => {
                if l <= 0.003_130_8 {
                    l * 12.92
                } else {
                    1.055 * l.max(0.0).powf(1.0 / 2.4) - 0.055
                }
            }
            Self::Xycc => {
                if l >= 0.0 {
                    ((l + 0.02) / 1.02).powf(1.0 / 2.4)
                } else {
                    -((-l / 1.03).powf(1.0 / 2.4))
                }
            }
            Self::Bt240 => {
                if l < SMPTE240_BREAK_LIGHT {
                    l * 4.0
                } else {
                    SMPTE240_ALPHA * l.max(0.0).powf(0.45) - (SMPTE240_ALPHA - 1.0)
                }
            }
            Self::Bt709 | Self::Bt601 => {
                if l < 0.018 {
                    l * 4.5
                } else {
                    1.099 * l.max(0.0).powf(0.45) - 0.099
                }
            }
        };
        Some(out as f32)
    }

    /// Relative-linear 1.0 in cd/m²: PQ is authored against 10 000, HLG
    /// against 1 000, and an SDR curve has no absolute meaning.
    pub fn full_scale_nits(self, sdr_peak: f32) -> f32 {
        match self {
            Self::Pq => PQ_PEAK_NITS,
            Self::Hlg => HLG_PEAK_NITS,
            _ => sdr_peak.max(1.0),
        }
    }

    /// Signal to absolute display luminance in cd/m².
    pub fn to_nits(self, signal: f32, sdr_peak: f32) -> Option<f32> {
        let l = self.eotf(signal)?;
        Some(match self {
            Self::Pq => l * PQ_PEAK_NITS,
            Self::Hlg => l * HLG_PEAK_NITS,
            _ => l * sdr_peak.max(1.0),
        })
    }

    /// Absolute display luminance to signal.
    pub fn from_nits(self, nits: f32, sdr_peak: f32) -> Option<f32> {
        let l = match self {
            Self::Pq => nits / PQ_PEAK_NITS,
            Self::Hlg => nits / HLG_PEAK_NITS,
            _ => nits / sdr_peak.max(1.0),
        };
        self.oetf(l)
    }

    /// Sample the curve into `entries` relative-linear values for 8-bit codes.
    pub fn table(self, entries: usize) -> Vec<f32> {
        (0..entries)
            .map(|i| {
                let v = i as f32 / (entries - 1) as f32;
                self.eotf(v).unwrap_or(v)
            })
            .collect()
    }
}

/// HLG inverse OETF: signal to normalised scene light (0..=1 for 0..=1 000 cd/m²).
pub fn hlg_inverse_oetf(signal: f32) -> f32 {
    let v = signal.clamp(0.0, 1.0);
    if v <= 0.5 {
        v * v / 3.0
    } else {
        (((v - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

/// HLG forward OETF: normalised scene light to signal.
pub fn hlg_oetf(scene: f32) -> f32 {
    let l = scene.clamp(0.0, 1.0);
    if l <= 1.0 / 12.0 {
        (3.0 * l).sqrt()
    } else {
        (HLG_A * (12.0 * l - HLG_B).ln() + HLG_C).clamp(0.0, 1.0)
    }
}

/// BT.2100-2 HLG opto-optical transfer function, on the luma it is driven by:
/// `Y_D = α·Y_S^γ`. α is a display gain in cd/m², and the standard's own
/// processing default is 1.0, which keeps 1.0 pinned to the panel's peak.
/// Earlier revisions added `+ β` (the black lift); BT.2100-2 dropped it in
/// favour of lifting the signal before the OOTF, see [`hlg_black_lift`].
pub fn hlg_ootf(scene: f32, system_gamma: f32) -> f32 {
    scene.max(0.0).powf(system_gamma)
}

/// BT.2100-2's in-signal black lift for a display with a non-zero floor, where
/// `luminance_ratio` is L_B/L_W. Zero for an ideal panel.
pub fn hlg_black_lift(luminance_ratio: f32, system_gamma: f32) -> f32 {
    if luminance_ratio <= 0.0 {
        return 0.0;
    }
    (3.0 * luminance_ratio.powf(1.0 / system_gamma)).sqrt()
}

/// BT.2100 system gamma for a given display peak: γ = 1.2 + 0.42·log10(Lw/1000).
///
/// BT.2100-3 states this form for Lw in 400..=2000 cd/m² and extends outside
/// that band with `1.2·1.111^log2(Lw/1000)`; the two disagree by 0.028 at
/// 4000 cd/m².
pub fn hlg_system_gamma(display_peak_nits: f32) -> f32 {
    1.2 + 0.42 * (display_peak_nits.max(1.0) / 1_000.0).log10()
}

/// Apply the HLG OOTF to a linear-light RGB triplet in scene-light form.
///
/// The OOTF is luminance-relative, so `kr`/`kb` must be those of the triplet's
/// own primaries. Returns display-referred light.
pub fn hlg_ootf_rgb(rgb: [f32; 3], kr: f32, kb: f32, system_gamma: f32) -> [f32; 3] {
    let y = rgb[0].mul_add(kr, rgb[2] * kb) + rgb[1] * (1.0 - kr - kb);
    if y <= 0.0 {
        return rgb;
    }
    let scaled = hlg_ootf(y, system_gamma) / y;
    [rgb[0] * scaled, rgb[1] * scaled, rgb[2] * scaled]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn pq_anchors_match_st_2084() {
        // SMPTE ST 2084's own code-value table, and BT.2408's reference points.
        for (nits, want) in [
            (0.005f32, 0.015_076_4),
            (1.0, 0.149_945_7),
            (100.0, 0.508_078_4),
            (203.0, 0.580_688_9),
            (262.0, 0.607_509_8),
            (1_000.0, 0.751_827_1),
            (4_000.0, 0.902_572_4),
            (10_000.0, 1.0),
        ] {
            let v = Transfer::Pq.from_nits(nits, SDR_PEAK_NITS).unwrap();
            assert!(close(v, want, 1e-6), "{nits} -> {v}");
            let rt = Transfer::Pq.to_nits(v, 0.0).unwrap();
            assert!(close(rt, nits, nits.max(1.0) * 1e-4), "{nits} -> {v} -> {rt}");
        }
        assert!(close(Transfer::Pq.eotf(0.0).unwrap(), 0.0, 1e-9));
    }

    #[test]
    fn hlg_curve_and_inverse_agree_with_bt_2100() {
        assert!(close(hlg_inverse_oetf(0.5), 1.0 / 12.0, 1e-7));
        assert!(close(hlg_inverse_oetf(1.0), 1.0, 1e-6));
        assert!(close(hlg_oetf(1.0 / 12.0), 0.5, 1e-7));
        assert!(close(hlg_oetf(1.0), 1.0, 1e-6));
        for s in [0.0f32, 0.1, 0.25, 0.5, 0.75, 1.0] {
            let rt = hlg_oetf(hlg_inverse_oetf(s));
            assert!(close(rt, s, 1e-6), "{s} -> {rt}");
        }
    }

    #[test]
    fn hlg_low_segment_is_not_twelve_times_too_hot() {
        // A factor-of-12 error here lifts every dark pixel into mid grey.
        assert!(close(hlg_inverse_oetf(0.25), 0.020_833_3, 1e-6));
    }

    #[test]
    fn sdr_curves_round_trip() {
        let all = [
            Transfer::Bt709,
            Transfer::Bt601,
            Transfer::Bt240,
            Transfer::Gamma22,
            Transfer::Gamma28,
            Transfer::Srgb,
            Transfer::Xycc,
        ];
        // Light → signal → light is exact everywhere, because each branch of
        // the inverse pair maps back onto the light branch it came from.
        for t in all {
            for i in 0..=1000u32 {
                let l = i as f32 / 1000.0;
                let rt = t.eotf(t.oetf(l).unwrap()).unwrap();
                assert!(close(rt, l, 3e-4), "{t:?} {l} -> {rt}");
            }
        }
        // Signal → light → signal additionally catches a branch whose cut the
        // forward curve crosses. xvYCC is meant to sit below black just above
        // signal 0 (that is the extended range), so it is excluded here and
        // covered by the direction above.
        for t in all {
            if t == Transfer::Xycc {
                continue;
            }
            for i in 0..=1000u32 {
                let v = i as f32 / 1000.0;
                let rt = t.oetf(t.eotf(v).unwrap()).unwrap();
                // Rec.709's own rounded constants disagree by 2.47e-4 at the
                // joint (see bt709_carries_the_standard_s_own_breakpoint_step),
                // so a round trip crossing it inherits that step.
                assert!(close(rt, v, 3e-4), "{t:?} {v} -> {rt}");
            }
        }
    }

    #[test]
    fn bt709_carries_the_standard_s_own_breakpoint_step() {
        // Rec.709 states 4.5L below L = 0.018 and 1.099L^0.45 - 0.099 above it.
        // Those rounded constants disagree by 2.5e-4 at the joint, which is the
        // standard's own inconsistency rather than a bug in either branch.
        let linear = 4.5 * 0.018;
        let power = 1.099 * 0.018f32.powf(0.45) - 0.099;
        assert!(close(Transfer::Bt709.eotf(0.080).unwrap(), 0.080 / 4.5, 1e-6));
        assert!((linear - power).abs() > 2e-4 && (linear - power).abs() < 3e-4);
    }

    #[test]
    fn bt240_is_continuous_and_above_bt709() {
        // 240M puts 18 % grey at a lower code than BT.709 and reads back more
        // light for the same signal. The exponent that used to live here broke
        // both that ordering and continuity at the joint.
        assert!(close(Transfer::Bt240.oetf(0.18).unwrap(), 0.402_247, 1e-6));
        assert!(close(Transfer::Bt709.oetf(0.18).unwrap(), 0.409_007_7, 1e-6));
        for l in [0.0227, 0.022_821_586, 0.0229] {
            let below = Transfer::Bt240.eotf(4.0 * l - 1e-5).unwrap();
            let above = Transfer::Bt240.eotf(4.0 * l + 1e-5).unwrap();
            assert!((below - above).abs() < 1e-4, "{l}: {below} vs {above}");
        }
        assert!(Transfer::Bt240.eotf(0.3).unwrap() > Transfer::Bt709.eotf(0.3).unwrap());
    }

    #[test]
    fn bt709_and_srgb_disagree_at_mid_grey() {
        let b = Transfer::Bt709.eotf(0.5).unwrap();
        let s = Transfer::Srgb.eotf(0.5).unwrap();
        assert!(close(b, 0.259_589_4, 1e-6), "{b}");
        assert!(close(s, 0.214_041_1, 1e-6), "{s}");
    }

    #[test]
    fn hdr_curves_round_trip_in_nits() {
        for (t, peaks) in [
            (Transfer::Pq, [1.0f32, 100.0, 1_000.0, 4_000.0, 10_000.0].as_slice()),
            (Transfer::Hlg, &[1.0, 100.0, 600.0, 1_000.0]),
        ] {
            for n in peaks {
                let v = t.from_nits(*n, SDR_PEAK_NITS).unwrap();
                let rt = t.to_nits(v, SDR_PEAK_NITS).unwrap();
                assert!(close(rt, *n, 0.05), "{t:?} {n} -> {v} -> {rt}");
            }
        }
    }

    #[test]
    fn codes_and_labels_cover_h273() {
        for code in [1u8, 4, 5, 6, 7, 8, 11, 12, 13, 14, 15, 16, 18] {
            let t = Transfer::from_code(code);
            assert_ne!(t, Transfer::Unknown, "{code}");
            assert!(!t.label().is_empty());
        }
        assert_eq!(Transfer::from_code(3), Transfer::Unknown);
        assert_eq!(Transfer::from_code(0), Transfer::Unknown);
        assert_eq!(Transfer::from_code(17), Transfer::Unknown);
        assert!(Transfer::Pq.is_hdr() && Transfer::Hlg.is_hdr());
        assert!(!Transfer::Bt709.is_hdr());
        assert_eq!(Transfer::from_code(16).code(), 16);
    }

    #[test]
    fn hlg_ootf_follows_bt_2100_2_form() {
        let g = hlg_system_gamma(1_000.0);
        assert!(close(g, 1.2, 1e-6), "{g}");
        // BT.2100-2 Note 5f's system gamma at the peaks displays ship at.
        for (nits, want) in [(400.0, 1.032_865), (2_000.0, 1.326_433), (4_000.0, 1.452_865)] {
            assert!(
                close(hlg_system_gamma(nits), want, 1e-5),
                "{nits} -> {}",
                hlg_system_gamma(nits)
            );
        }
        assert!(close(hlg_ootf(0.0, g), 0.0, 1e-9));
        assert!(close(hlg_ootf(1.0, g), 1.0, 1e-6));
        let mid = hlg_ootf(0.5, 1.2);
        assert!(mid > 0.4 && mid < 0.5, "{mid}");
        assert!(close(hlg_ootf(0.5, 1.0), 0.5, 1e-6));
        // The OOTF scales the whole triplet by a luma-driven gain, so chroma
        // ratios survive untouched.
        let rgb = [0.4f32, 0.05, 0.02];
        let out = hlg_ootf_rgb(rgb, 0.2627, 0.0593, 1.2);
        for i in 1..3 {
            assert!(
                close(out[i] / out[0], rgb[i] / rgb[0], 1e-5),
                "chroma drifted: {out:?}"
            );
        }
        let y = 0.2627 * rgb[0] + 0.678_0 * rgb[1] + 0.0593 * rgb[2];
        assert!(close(out[0] / rgb[0], y.powf(0.2), 1e-5), "{out:?}");
    }

    #[test]
    fn hlg_black_lift_matches_the_signal_domain_form() {
        // β = sqrt(3·(L_B/L_W)^(1/γ)) from BT.2100-2; an ideal panel lifts nothing.
        assert!(close(hlg_black_lift(0.0, 1.2), 0.0, 1e-9));
        let ratio = 0.005 / 1_000.0;
        let gamma = hlg_system_gamma(1_000.0);
        let lifted = hlg_black_lift(ratio, gamma);
        // The lift is defined so that decoding β yields exactly the panel's
        // black floor in scene light, which is the only check that pins it.
        let scene = hlg_inverse_oetf(lifted);
        assert!(
            close(scene, ratio.powf(1.0 / gamma), 1e-7),
            "{lifted} decodes to {scene}"
        );
        assert!(lifted > 0.0 && lifted < 0.5, "{lifted}");
    }

    #[test]
    fn table_is_monotonic_and_bounded() {
        for t in [Transfer::Bt709, Transfer::Pq, Transfer::Hlg, Transfer::Srgb] {
            let tab = t.table(256);
            assert_eq!(tab.len(), 256);
            assert!(close(*tab.first().unwrap(), 0.0, 1e-6));
            for w in tab.windows(2) {
                assert!(w[1] >= w[0], "{t:?}");
            }
        }
    }
}
