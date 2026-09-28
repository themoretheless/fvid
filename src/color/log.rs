//! Camera log transfer curves: Sony S-Log, Canon Log, Panasonic V-Log, ARRI LogC.
//!
//! Each curve is transcribed from the vendor's own specification and pinned by
//! that specification's published code-value anchors (see the tests), so a
//! decoded pixel can be checked against the camera manufacturer's tables
//! rather than against a fitting. Curves whose constants are not published by
//! the vendor — and so cannot be checked — are deliberately absent.
//!
//! Two normalisations are in play and they are not the same:
//!
//! * **signal** — the curve's own output, normalised over the code range the
//!   vendor authored it for ([`Log::codes`] gives that range at 10-bit).
//! * **linear** — scene exposure or reflectance, where 1.0 is 100 % diffuse
//!   white. Some vendors express their curve against IRE, where 100 % reflectance
//!   sits at 0.9; [`Log::to_linear`] undoes that so every profile speaks the
//!   same units on the linear side.

use std::f64::consts::LN_2;

use crate::color::primaries::Primaries;

/// Sony and Canon author their curves against IRE: 100 % reflectance = 0.9.
const IRE_FROM_REFLECTANCE: f64 = 0.9;

/// A vendor-published camera log curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Log {
    /// Sony S-Log1 (S-Log2 Technical Paper V1.0, without the 155/219 scale).
    SLog1,
    /// Sony S-Log2 (S-Log2 Technical Paper V1.0).
    SLog2,
    /// Sony S-Log3 (Technical Summary for S-Gamut3.Cine/S-Log3, V1.00).
    SLog3,
    /// Canon Log / Log 1 (CANON-LOG TRANSFER CHARACTERISTIC, v1.2 legal form).
    CLog,
    /// Canon Log 2 (Canon input-transform package).
    CLog2,
    /// Canon Log 3 (Canon input-transform package).
    CLog3,
    /// Panasonic V-Log (VARICAM V-Log/V-Gamut Reference Manual rev 1.0).
    VLog,
    /// ARRI ALEXA Log C at EI 800 (ALEXA Log C Curve – Usage in VFX).
    LogC,
    /// ARRI LogC4 for ALEXA 35 (ARRI LogC4 Specification, EI-independent).
    LogC4,
}

/// Where a profile's normalised signal sits in an integer code range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Codes {
    /// Code value at signal 0.0 for 10-bit material.
    pub black: f32,
    /// Code span from signal 0.0 to signal 1.0 for 10-bit material.
    pub span: f32,
}

// Sony S-Log1/S-Log2: shared offset and slope, S-Log2 scales the input.
const SLOG_A: f64 = 0.432_699;
const SLOG_B: f64 = 0.616_596;
const SLOG_C: f64 = 0.03;
const SLOG_D: f64 = 0.037_584;
/// Breakpoint of the linear toe, which is also where the log branch starts.
const SLOG_LIN: f64 = 0.030_001_222_851_889_303;
const SLOG2_NEG_SLOPE: f64 = 3.538_812_785_388_13;
const SLOG1_NEG_SLOPE: f64 = 5.0;
const SLOG2_SCALE: f64 = 155.0 / 219.0;

// Sony S-Log3.
const SLOG3_BLACK: f64 = 95.0;
const SLOG3_GREY: f64 = 420.0;
const SLOG3_SLOPE: f64 = 261.5;
const SLOG3_OFFSET: f64 = 0.01;
const SLOG3_GREY_REF: f64 = 0.19;
const SLOG3_TOE_LINEAR: f64 = 0.011_25;
const SLOG3_TOE_END: f64 = 171.210_294_692_9;
const SLOG3_FULL: f64 = 1023.0;

// Canon Log 1 (v1.2 legal form), Log 2 and Log 3.
const CLOG_SLOPE: f64 = 0.453_101_79;
const CLOG_K: f64 = 10.159_6;
const CLOG_OFFSET: f64 = 0.125_122_48;
const CLOG2_SLOPE: f64 = 0.241_360_77;
const CLOG2_K: f64 = 87.099_375_46;
const CLOG2_OFFSET: f64 = 0.092_864_125;
const CLOG3_SLOPE: f64 = 0.367_268_45;
const CLOG3_K: f64 = 14.983_25;
const CLOG3_OFFSET: f64 = 0.122_405_37;
/// Canon grafts a linear toe onto ±0.014 IRE.
const CLOG3_TOE_X: f64 = 0.014;
const CLOG3_TOE_SLOPE: f64 = 1.975_479_8;
const CLOG3_TOE_OFFSET: f64 = 0.125_122_19;
const CLOG3_NEG_OFFSET: f64 = 0.127_839_01;

// Panasonic V-Log.
const VLOG_A: f64 = 0.241_514;
const VLOG_B: f64 = 0.008_73;
const VLOG_D: f64 = 0.598_206;
/// The two segments meet at reflectance 0.01, i.e. signal 0.181.
const VLOG_TOE_X: f64 = 0.01;
const VLOG_TOE_SLOPE: f64 = 5.6;
const VLOG_TOE_SIGNAL: f64 = 0.125;

// ARRI Log C, EI 800 exposure form.
const LOGC_A: f64 = 5.555_556;
const LOGC_B: f64 = 0.052_272;
const LOGC_C: f64 = 0.247_190;
const LOGC_D: f64 = 0.385_537;
const LOGC_E: f64 = 5.367_655;
const LOGC_F: f64 = 0.092_809;
const LOGC_CUT: f64 = 0.010_591;

// ARRI LogC4. 117.45 is the rounded value ARRI prescribes for the slope.
const LOGC4_A: f64 = (262_144.0 - 16.0) / 117.45;
const LOGC4_B: f64 = (1023.0 - 95.0) / 1023.0;
const LOGC4_C: f64 = 95.0 / 1023.0;

impl Log {
    pub const ALL: [Log; 9] = [
        Log::SLog1,
        Log::SLog2,
        Log::SLog3,
        Log::CLog,
        Log::CLog2,
        Log::CLog3,
        Log::VLog,
        Log::LogC,
        Log::LogC4,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SLog1 => "slog1",
            Self::SLog2 => "slog2",
            Self::SLog3 => "slog3",
            Self::CLog => "clog",
            Self::CLog2 => "clog2",
            Self::CLog3 => "clog3",
            Self::VLog => "vlog",
            Self::LogC => "logc",
            Self::LogC4 => "logc4",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.label() == label)
    }

    /// The camera's own working gamut, when the vendor publishes its primaries.
    ///
    /// `None` means the curve is citable but the matching gamut's coordinates
    /// are not: S-Log1/2 predate a published S-Gamut table, and Canon draws
    /// Cinema Gamut as a figure rather than a set of numbers. Log material from
    /// those profiles arrives in a container gamut the demuxer already knows,
    /// so the caller should convert from the source it is given rather than
    /// from a guessed triangle.
    pub fn gamut(self) -> Option<Primaries> {
        Some(match self {
            Self::SLog3 => Primaries::S_GAMUT3,
            Self::VLog => Primaries::V_GAMUT,
            Self::LogC => Primaries::ALEX3_WIDE,
            // ARRI's LogC4 specification makes ARRI Wide Gamut 4 part of the
            // definition rather than an option, so the curve has to hand its
            // pixels to those primaries and to nothing else.
            Self::LogC4 => Primaries::ALEX3_EXPANDED,
            Self::SLog1 | Self::SLog2 | Self::CLog | Self::CLog2 | Self::CLog3 => return None,
        })
    }

    /// The code range the vendor authored this curve over, at 10-bit.
    ///
    /// S-Log1/2 record legal range (64..940 of 1023) while every other profile
    /// here is full range, which is the single most common way log material is
    /// mis-decoded.
    pub fn codes(self) -> Codes {
        match self {
            Self::SLog1 | Self::SLog2 => Codes {
                black: 64.0,
                span: 876.0,
            },
            _ => Codes {
                black: 0.0,
                span: 1023.0,
            },
        }
    }

    /// Integer code value → normalised signal, for `bits`-bit material.
    pub fn signal_from_code(self, code: f32, bits: u32) -> f32 {
        let scale = codes_scale(bits);
        let codes = self.codes();
        (code - codes.black * scale) / (codes.span * scale)
    }

    /// Normalised signal → code value, left fractional for the caller to round.
    pub fn code_from_signal(self, signal: f32, bits: u32) -> f32 {
        let scale = codes_scale(bits);
        let codes = self.codes();
        signal * (codes.span * scale) + codes.black * scale
    }

    /// True when this profile's signal is authored against IRE rather than
    /// reflectance, i.e. 100 % white sits at signal input 0.9 → IRE 1.0.
    fn uses_ire(self) -> bool {
        matches!(
            self,
            Self::SLog1 | Self::SLog2 | Self::CLog | Self::CLog2 | Self::CLog3
        )
    }

    /// Normalised signal → scene linear, 1.0 = 100 % diffuse white.
    ///
    /// Signals below the toe return negative exposure, which is intended: log
    /// curves carry sub-black sensor noise, and clamping it would flatten the
    /// very shadow detail the profile exists to preserve.
    pub fn to_linear(self, signal: f32) -> f32 {
        let y = signal as f64;
        let x = match self {
            Self::SLog1 | Self::SLog2 => {
                let scale = match self {
                    Self::SLog2 => SLOG2_SCALE,
                    _ => 1.0,
                };
                let neg = match self {
                    Self::SLog2 => SLOG2_NEG_SLOPE,
                    _ => SLOG1_NEG_SLOPE,
                };
                if y > SLOG_LIN {
                    (10f64.powf((y - SLOG_B - SLOG_C) / SLOG_A) - SLOG_D) / scale
                } else {
                    (y - SLOG_LIN) / neg
                }
            }
            Self::SLog3 => {
                let cv = y * SLOG3_FULL;
                if cv >= SLOG3_TOE_END {
                    10f64.powf((cv - SLOG3_GREY) / SLOG3_SLOPE) * SLOG3_GREY_REF - SLOG3_OFFSET
                } else {
                    (cv - SLOG3_BLACK) * SLOG3_TOE_LINEAR / (SLOG3_TOE_END - SLOG3_BLACK)
                }
            }
            Self::CLog => log_side(y, CLOG_SLOPE, CLOG_K, CLOG_OFFSET),
            Self::CLog2 => log_side(y, CLOG2_SLOPE, CLOG2_K, CLOG2_OFFSET),
            Self::CLog3 => clog3_decode(y),
            Self::VLog => {
                if y >= VLOG_TOE_SIGNAL + VLOG_TOE_SLOPE * VLOG_TOE_X {
                    10f64.powf((y - VLOG_D) / VLOG_A) - VLOG_B
                } else {
                    (y - VLOG_TOE_SIGNAL) / VLOG_TOE_SLOPE
                }
            }
            Self::LogC => {
                if y > LOGC_E * LOGC_CUT + LOGC_F {
                    (10f64.powf((y - LOGC_D) / LOGC_C) - LOGC_B) / LOGC_A
                } else {
                    (y - LOGC_F) / LOGC_E
                }
            }
            Self::LogC4 => {
                let (s, t) = logc4_toe();
                if y >= 0.0 {
                    (2f64.powf(14.0 * (y - LOGC4_C) / LOGC4_B + 6.0) - 64.0) / LOGC4_A
                } else {
                    y * s + t
                }
            }
        };
        let reflection = if self.uses_ire() {
            x * IRE_FROM_REFLECTANCE
        } else {
            x
        };
        reflection as f32
    }

    /// Scene linear → normalised signal. Inverse of [`Log::to_linear`] up to
    /// float rounding.
    pub fn from_linear(self, linear: f32) -> f32 {
        let mut x = if self.uses_ire() {
            linear as f64 / IRE_FROM_REFLECTANCE
        } else {
            linear as f64
        };
        // The log branch cannot survive a non-positive argument; the toe takes
        // over there, which is what the vendors specify for sub-black material.
        if !x.is_finite() {
            return f32::NAN;
        }
        x = match self {
            Self::SLog1 | Self::SLog2 => {
                let scale = match self {
                    Self::SLog2 => SLOG2_SCALE,
                    _ => 1.0,
                };
                if x >= 0.0 {
                    SLOG_A * (x * scale + SLOG_D).log10() + SLOG_B + SLOG_C
                } else {
                    x * match self {
                        Self::SLog2 => SLOG2_NEG_SLOPE,
                        _ => SLOG1_NEG_SLOPE,
                    } + SLOG_LIN
                }
            }
            Self::SLog3 => {
                if x >= SLOG3_TOE_LINEAR {
                    (SLOG3_GREY + SLOG3_SLOPE * ((x + SLOG3_OFFSET) / SLOG3_GREY_REF).log10())
                        / SLOG3_FULL
                } else {
                    (x * (SLOG3_TOE_END - SLOG3_BLACK) / SLOG3_TOE_LINEAR + SLOG3_BLACK)
                        / SLOG3_FULL
                }
            }
            Self::CLog => log_pair(x, CLOG_SLOPE, CLOG_K, CLOG_OFFSET),
            Self::CLog2 => log_pair(x, CLOG2_SLOPE, CLOG2_K, CLOG2_OFFSET),
            Self::CLog3 => {
                if x > CLOG3_TOE_X {
                    CLOG3_SLOPE * (CLOG3_K * x + 1.0).log10() + CLOG3_OFFSET
                } else if x < -CLOG3_TOE_X {
                    -CLOG3_SLOPE * (-CLOG3_K * x + 1.0).log10() + CLOG3_NEG_OFFSET
                } else {
                    CLOG3_TOE_SLOPE * x + CLOG3_TOE_OFFSET
                }
            }
            Self::VLog => {
                if x < VLOG_TOE_X {
                    VLOG_TOE_SLOPE * x + VLOG_TOE_SIGNAL
                } else {
                    VLOG_A * (x + VLOG_B).log10() + VLOG_D
                }
            }
            Self::LogC => {
                if x > LOGC_CUT {
                    LOGC_C * (LOGC_A * x + LOGC_B).log10() + LOGC_D
                } else {
                    LOGC_E * x + LOGC_F
                }
            }
            Self::LogC4 => {
                let (s, t) = logc4_toe();
                if x >= t {
                    ((LOGC4_A * x + 64.0).log2() - 6.0) / 14.0 * LOGC4_B + LOGC4_C
                } else {
                    (x - t) / s
                }
            }
        };
        x as f32
    }

    /// Sample the decoder: `entries` evenly spaced signals across 0..=1 → linear.
    pub fn table(self, entries: usize) -> Vec<f32> {
        let entries = entries.max(2);
        (0..entries)
            .map(|i| self.to_linear(i as f32 / (entries - 1) as f32))
            .collect()
    }
}

/// Sony S-Log1/2 and Canon Log put negative exposure on a mirrored log branch.
fn log_side(y: f64, slope: f64, k: f64, offset: f64) -> f64 {
    let d = y - offset;
    if d >= 0.0 {
        (10f64.powf(d / slope) - 1.0) / k
    } else {
        (1.0 - 10f64.powf(-d / slope)) / k
    }
}

fn log_pair(x: f64, slope: f64, k: f64, offset: f64) -> f64 {
    if x >= 0.0 {
        slope * (k * x + 1.0).log10() + offset
    } else {
        -slope * (-k * x + 1.0).log10() + offset
    }
}

/// Canon Log 3's three segments, decoded by signal threshold. The published
/// constants join the toe and the log branch to within 1.2e-5, so the toe's own
/// graft points decide which side a signal belongs to.
fn clog3_decode(y: f64) -> f64 {
    let upper = CLOG3_TOE_SLOPE * CLOG3_TOE_X + CLOG3_TOE_OFFSET;
    let lower = CLOG3_TOE_OFFSET - CLOG3_TOE_SLOPE * CLOG3_TOE_X;
    if y >= upper {
        (10f64.powf((y - CLOG3_OFFSET) / CLOG3_SLOPE) - 1.0) / CLOG3_K
    } else if y <= lower {
        (1.0 - 10f64.powf((CLOG3_NEG_OFFSET - y) / CLOG3_SLOPE)) / CLOG3_K
    } else {
        (y - CLOG3_TOE_OFFSET) / CLOG3_TOE_SLOPE
    }
}

/// LogC4's linear-toe slope and its breakpoint, both derived from the
/// specification's `a`, `b` and `c` so the pair stays continuous.
fn logc4_toe() -> (f64, f64) {
    let s = 7.0 * LN_2 * 2f64.powf(7.0 - 14.0 * LOGC4_C / LOGC4_B) / (LOGC4_A * LOGC4_B);
    let t = (2f64.powf(14.0 * (-LOGC4_C / LOGC4_B) + 6.0) - 64.0) / LOGC4_A;
    (s, t)
}

/// 10-bit code values scale by one step per added bit.
fn codes_scale(bits: u32) -> f32 {
    (1u32 << bits.saturating_sub(10).min(14)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    /// 10-bit code value a vendor's own table quotes for a given reflectance.
    fn code_at(profile: Log, reflectance: f32, bits: u32) -> f32 {
        profile.code_from_signal(profile.from_linear(reflectance), bits)
    }

    #[test]
    fn sony_s_log3_hits_sony_published_codes() {
        // Technical Summary V1.00: 0 % → 95, 18 % grey → 420, 90 % → 598.
        assert!(close(code_at(Log::SLog3, 0.0, 10), 95.0, 0.02), "black");
        assert!(close(code_at(Log::SLog3, 0.18, 10), 420.0, 0.02), "grey");
        assert!(close(code_at(Log::SLog3, 0.90, 10), 598.0, 0.5), "white");
    }

    #[test]
    fn sony_s_log_codes_follow_the_legal_range_table() {
        // S-Log2 Technical Paper: 0 % → 90, 18 % → 347, 90 % → 582; S-Log1
        // 0 % → 90, 18 % → 394, 90 % → 636. Both are legal range, so a code
        // of 0 reflectance must land above the 64 pedestal, not on it.
        for (profile, want) in [
            (Log::SLog2, [90.0, 347.0, 582.0]),
            (Log::SLog1, [90.0, 394.0, 636.0]),
        ] {
            let got = [
                code_at(profile, 0.0, 10),
                code_at(profile, 0.18, 10),
                code_at(profile, 0.90, 10),
            ];
            for (g, w) in got.iter().zip(want) {
                assert!(close(*g, w, 0.6), "{profile:?} {g} vs {w}");
            }
            let (black, span) = (profile.codes().black, profile.codes().span);
            assert_eq!((black, span), (64.0, 876.0), "{profile:?}");
        }
    }

    #[test]
    fn canon_log_codes_match_the_canon_tables() {
        // Canon Log 1 publishes 0 % → 128, 18 % → 351, 90 % → 614 at 10-bit
        // (fig 11). Log 2 and Log 3 have no public figure table, so they are
        // pinned by the black and grey points their IDT constants imply.
        for (profile, want) in [
            (
                Log::CLog,
                [(0.0f32, 128.0), (0.18, 351.0), (0.90, 614.0)].as_slice(),
            ),
            (Log::CLog2, [(0.0f32, 95.0), (0.18, 407.0)].as_slice()),
            (Log::CLog3, [(0.0f32, 128.0), (0.18, 351.3)].as_slice()),
        ] {
            for &(reflectance, w) in want {
                let got = code_at(profile, reflectance, 10);
                assert!(
                    close(got, w, 0.7),
                    "{profile:?} {reflectance} → {got} vs {w}"
                );
            }
        }
    }

    #[test]
    fn panasonic_v_log_hits_the_varicam_table() {
        // V-Log/V-Gamut Reference Manual fig 2.2: 0 % → 128, 18 % → 433,
        // 90 % → 602, and 12-bit is exactly four times the 10-bit code.
        for (reflectance, want) in [(0.0f32, 128.0), (0.18, 433.0), (0.90, 602.0)] {
            let got = code_at(Log::VLog, reflectance, 10);
            assert!(close(got, want, 0.7), "{reflectance} → {got} vs {want}");
            let wide = code_at(Log::VLog, reflectance, 12);
            assert!(close(wide, got * 4.0, 0.1), "{reflectance} 12-bit {wide}");
        }
    }

    #[test]
    fn arri_log_c_is_normalised_to_grey_at_every_reference_point() {
        // ALEXA Log C: 18 % grey → 0.391 → code 400, by design of the curve.
        let grey = Log::LogC.from_linear(0.18);
        assert!(close(grey, 0.391_34, 1e-3), "{grey}");
        assert!(close(code_at(Log::LogC, 0.18, 10), 400.0, 0.6));
        // LogC4 puts 18 % grey at 0.2784 (12-bit legal 1232).
        let grey4 = Log::LogC4.from_linear(0.18);
        assert!(close(grey4, 0.278_4, 3e-4), "logc4 {grey4}");
        assert!(close(Log::LogC4.from_linear(0.0), 0.092_864, 1e-5));
    }

    /// ARRI's LogC4 specification §4.3 defines LogC4 as the LogC4 curve *and*
    /// ARRI Wide Gamut 4, so a decoded LogC4 pixel has to leave the curve in
    /// those primaries. Before this, it left in the gamut LogC uses, and the
    /// wrong triangle recoloured it without touching luminance: measured
    /// through `primaries::rgb_to_rgb(…, BT709)`, a skin tone moves 12.7 8-bit
    /// steps and a saturated leg of the triangle 42–70, while white and 18 %
    /// grey do not move at all — which is why no brightness check caught it.
    #[test]
    fn log_c_4_unwraps_in_the_gamut_its_specification_names() {
        assert_eq!(Log::LogC4.gamut(), Some(Primaries::ALEX3_EXPANDED));
        // LogC keeps the gamut its curve was transcribed with; ARRI prints no
        // vendor matrix for it here, so nothing else is claimed.
        assert_eq!(Log::LogC.gamut(), Some(Primaries::ALEX3_WIDE));
        for profile in [Log::SLog1, Log::SLog2, Log::CLog, Log::CLog2, Log::CLog3] {
            assert_eq!(profile.gamut(), None, "{profile:?}");
        }
        assert_eq!(Log::VLog.gamut(), Some(Primaries::V_GAMUT));
        assert_eq!(Log::SLog3.gamut(), Some(Primaries::S_GAMUT3));
    }

    /// Every code a camera writes has to come back to itself: the claim a log
    /// profile exists for is not that a curve bends the right way at the points
    /// a vendor's table quotes, but that the whole integer range survives the
    /// walk out of the codes, through linear light, and back. Swept at 10 and 12
    /// bits and over the legal range in both directions, since a clip's own
    /// blanks sit below the pedestal and an extended recording above the top.
    /// Worst measured error 6.1e-5 of a code at 10 bits and 2.4e-4 at 12, both
    /// at code 949 of the S-Log curves, at the end of the range where a signal
    /// is largest and f32 is coarsest; no other profile exceeds 3.3e-5.
    ///
    /// Walking integers rather than sampling 201 points is what makes this more
    /// than a repeat of `every_profile_round_trips_over_its_own_domain`: the
    /// steps of a coarse sample land between the branch seams, while an exact
    /// code hits them. Drifting one S-Log3 constant by a part in 10 000 leaves
    /// the sampled test green and fails this one at code 172, the toe seam,
    /// because decode then lands on the other branch of the curve.
    #[test]
    fn every_code_a_camera_writes_comes_back_to_itself() {
        for profile in Log::ALL {
            for bits in [10u32, 12] {
                for code in 0..=(1u32 << bits) - 1 {
                    let signal = profile.signal_from_code(code as f32, bits);
                    let linear = profile.to_linear(signal);
                    let back = profile.code_from_signal(profile.from_linear(linear), bits);
                    assert!(
                        (back - code as f32).abs() <= 2e-3,
                        "{profile:?} {bits}-bit code {code} → {signal} → {linear} → {back}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_profile_round_trips_over_its_own_domain() {
        for profile in Log::ALL {
            for i in 0..=200u32 {
                let signal = i as f32 / 200.0;
                let linear = profile.to_linear(signal);
                let back = profile.from_linear(linear);
                assert!(
                    back.is_finite() && close(back, signal, 2e-4),
                    "{profile:?} {signal} → {linear} → {back}"
                );
            }
        }
    }

    #[test]
    fn every_profile_is_monotonic_and_greys_get_darker_downwards() {
        for profile in Log::ALL {
            let tab = profile.table(256);
            assert_eq!(tab.len(), 256);
            for w in tab.windows(2) {
                assert!(w[1] >= w[0], "{profile:?} {w:?}");
            }
            assert!(
                profile.to_linear(0.0) < profile.to_linear(0.5),
                "{profile:?}"
            );
            assert!(
                profile.to_linear(0.5) < profile.to_linear(1.0),
                "{profile:?}"
            );
        }
    }

    #[test]
    fn log_profiles_reach_far_above_diffuse_white() {
        // The point of a log curve is highlight headroom: 100 % white must sit
        // far below the top of the code range, unlike a video gamma curve.
        for profile in Log::ALL {
            let white = profile.from_linear(1.0);
            let top = profile.from_linear(8.0);
            assert!(white < top, "{profile:?} {white} {top}");
            assert!(white < 0.75, "{profile:?} white at {white}");
            assert!(profile.to_linear(white) > 0.999, "{profile:?}");
        }
    }

    #[test]
    fn labels_round_trip() {
        for profile in Log::ALL {
            assert_eq!(
                Log::from_label(profile.label()),
                Some(profile),
                "{profile:?}"
            );
        }
        assert_eq!(Log::from_label("n-Log"), None);
    }

    #[test]
    fn code_mapping_scales_with_bit_depth() {
        for profile in Log::ALL {
            let (black, span) = (profile.codes().black, profile.codes().span);
            let at_10 = profile.code_from_signal(0.5, 10);
            let at_12 = profile.code_from_signal(0.5, 12);
            assert!(
                close(at_12, at_10 * 4.0, 0.5),
                "{profile:?} {at_10} {at_12}"
            );
            let back = profile.signal_from_code(at_10, 10);
            assert!(close(back, 0.5, 1e-4), "{profile:?} {back}");
            assert!(black <= 64.0 && span > 0.0, "{profile:?}");
        }
    }
}
