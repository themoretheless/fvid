//! Colour primaries, white points, RGB/YUV matrices and chromatic adaptation.

/// CIE 1931 chromaticity of one colour channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chromaticity {
    pub x: f64,
    pub y: f64,
}

/// An RGB colour space: three primaries plus a white point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primaries {
    pub r: Chromaticity,
    pub g: Chromaticity,
    pub b: Chromaticity,
    pub white: Chromaticity,
}

impl Primaries {
    pub const BT709: Primaries = Primaries {
        r: Chromaticity { x: 0.640, y: 0.330 },
        g: Chromaticity { x: 0.300, y: 0.600 },
        b: Chromaticity { x: 0.150, y: 0.060 },
        white: WHITE_D65,
    };
    pub const BT2020: Primaries = Primaries {
        r: Chromaticity { x: 0.708, y: 0.292 },
        g: Chromaticity { x: 0.170, y: 0.797 },
        b: Chromaticity { x: 0.131, y: 0.046 },
        white: WHITE_D65,
    };
    pub const BT601_EBU: Primaries = Primaries {
        r: Chromaticity { x: 0.630, y: 0.340 },
        g: Chromaticity { x: 0.295, y: 0.605 },
        b: Chromaticity { x: 0.155, y: 0.077 },
        white: WHITE_D65,
    };
    pub const SMPTE170M: Primaries = Primaries {
        r: Chromaticity { x: 0.630, y: 0.340 },
        g: Chromaticity { x: 0.310, y: 0.595 },
        b: Chromaticity { x: 0.155, y: 0.077 },
        white: WHITE_D65,
    };
    pub const BT470M: Primaries = Primaries {
        r: Chromaticity { x: 0.670, y: 0.330 },
        g: Chromaticity { x: 0.210, y: 0.710 },
        b: Chromaticity { x: 0.140, y: 0.080 },
        white: WHITE_C,
    };
    /// SMPTE 240M reuses SMPTE 170M's primaries on a D65 white.
    pub const SMPTE240M: Primaries = Primaries {
        r: Chromaticity { x: 0.630, y: 0.340 },
        g: Chromaticity { x: 0.310, y: 0.595 },
        b: Chromaticity { x: 0.155, y: 0.077 },
        white: WHITE_D65,
    };
    pub const BT470BG: Primaries = Primaries {
        r: Chromaticity { x: 0.640, y: 0.330 },
        g: Chromaticity { x: 0.290, y: 0.600 },
        b: Chromaticity { x: 0.150, y: 0.060 },
        white: WHITE_D65,
    };
    pub const DCI_P3: Primaries = Primaries {
        r: Chromaticity { x: 0.680, y: 0.320 },
        g: Chromaticity { x: 0.265, y: 0.690 },
        b: Chromaticity { x: 0.150, y: 0.060 },
        white: WHITE_DCI,
    };
    pub const DISPLAY_P3: Primaries = Primaries {
        r: Chromaticity { x: 0.680, y: 0.320 },
        g: Chromaticity { x: 0.265, y: 0.690 },
        b: Chromaticity { x: 0.150, y: 0.060 },
        white: WHITE_D65,
    };
    pub const ADOBE_RGB: Primaries = Primaries {
        r: Chromaticity { x: 0.640, y: 0.330 },
        g: Chromaticity { x: 0.210, y: 0.710 },
        b: Chromaticity { x: 0.150, y: 0.060 },
        white: WHITE_D65,
    };

    /// Luminance weight of each primary, derived from the chromaticities.
    pub fn kr_kb(&self) -> (f64, f64) {
        let s = self.scales();
        let sum = s[0] + s[1] + s[2];
        (s[0] / sum, s[2] / sum)
    }

    pub fn kg(&self) -> f64 {
        let (kr, kb) = self.kr_kb();
        1.0 - kr - kb
    }

    /// Map an H.273 / AV1 `colour_primaries` code.
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::BT709,
            4 => Self::BT470M,
            5 => Self::BT470BG,
            6 => Self::SMPTE170M,
            7 => Self::SMPTE240M,
            9 => Self::BT2020,
            11 => Self::DCI_P3,
            12 => Self::DISPLAY_P3,
            _ => return None,
        })
    }

    /// The code that signals these primaries, when the table is known.
    pub fn code(self) -> Option<u8> {
        if self == Self::BT709 {
            Some(1)
        } else if self == Self::BT470M {
            Some(4)
        } else if self == Self::BT470BG {
            Some(5)
        } else if self == Self::SMPTE170M {
            Some(6)
        } else if self == Self::SMPTE240M {
            Some(7)
        } else if self == Self::BT2020 {
            Some(9)
        } else if self == Self::DCI_P3 {
            Some(11)
        } else if self == Self::DISPLAY_P3 {
            Some(12)
        } else {
            None
        }
    }

    pub fn label(self) -> &'static str {
        if self == Self::BT709 {
            "BT.709"
        } else if self == Self::BT2020 {
            "BT.2020"
        } else if self == Self::BT470BG {
            "BT.470 System B/G"
        } else if self == Self::SMPTE170M {
            "SMPTE 170M"
        } else if self == Self::SMPTE240M {
            "SMPTE 240M"
        } else if self == Self::BT470M {
            "BT.470 System M"
        } else if self == Self::DCI_P3 {
            "DCI-P3"
        } else if self == Self::DISPLAY_P3 {
            "Display P3"
        } else if self == Self::ADOBE_RGB {
            "Adobe RGB"
        } else if self == Self::BT601_EBU {
            "BT.601 EBU"
        } else {
            "custom"
        }
    }

    /// Cone-response scales solving `M·S = white` with each primary at Y = 1.
    fn scales(&self) -> [f64; 3] {
        let w = xyz_of(self.white);
        let cr = xyz_of(self.r);
        let cg = xyz_of(self.g);
        let cb = xyz_of(self.b);
        let det = det3(cr, cg, cb);
        [
            det3(w, cg, cb) / det,
            det3(cr, w, cb) / det,
            det3(cr, cg, w) / det,
        ]
    }

    /// Row-major 3×3 matrix mapping linear RGB of these primaries to XYZ D65-relative.
    pub fn rgb_to_xyz(&self) -> [[f64; 3]; 3] {
        let s = self.scales();
        let rows = [xyz_of(self.r), xyz_of(self.g), xyz_of(self.b)];
        [
            [rows[0][0] * s[0], rows[1][0] * s[1], rows[2][0] * s[2]],
            [rows[0][1] * s[0], rows[1][1] * s[1], rows[2][1] * s[2]],
            [rows[0][2] * s[0], rows[1][2] * s[1], rows[2][2] * s[2]],
        ]
    }

    pub fn xyz_to_rgb(&self) -> [[f64; 3]; 3] {
        inv3(self.rgb_to_xyz())
    }
}

pub const WHITE_D65: Chromaticity = Chromaticity { x: 0.3127, y: 0.3290 };
pub const WHITE_DCI: Chromaticity = Chromaticity { x: 0.314, y: 0.351 };
pub const WHITE_D93: Chromaticity = Chromaticity { x: 0.285, y: 0.293 };
pub const WHITE_C: Chromaticity = Chromaticity { x: 0.310, y: 0.316 };

fn xyz_of(c: Chromaticity) -> [f64; 3] {
    [c.x / c.y, 1.0, (1.0 - c.x - c.y) / c.y]
}

type Col = [f64; 3];

fn det3(c0: Col, c1: Col, c2: Col) -> f64 {
    c0[0] * (c1[1] * c2[2] - c1[2] * c2[1])
        - c0[1] * (c1[0] * c2[2] - c1[2] * c2[0])
        + c0[2] * (c1[0] * c2[1] - c1[1] * c2[0])
}

fn inv3(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let c = [
        [
            m[1][1] * m[2][2] - m[1][2] * m[2][1],
            m[1][2] * m[2][0] - m[1][0] * m[2][2],
            m[1][0] * m[2][1] - m[1][1] * m[2][0],
        ],
        [
            m[0][2] * m[2][1] - m[0][1] * m[2][2],
            m[0][0] * m[2][2] - m[0][2] * m[2][0],
            m[0][1] * m[2][0] - m[0][0] * m[2][1],
        ],
        [
            m[0][1] * m[1][2] - m[0][2] * m[1][1],
            m[0][2] * m[1][0] - m[0][0] * m[1][2],
            m[0][0] * m[1][1] - m[0][1] * m[1][0],
        ],
    ];
    let det = m[0][0] * c[0][0] + m[0][1] * c[0][1] + m[0][2] * c[0][2];
    let inv = 1.0 / det;
    [
        [c[0][0] * inv, c[1][0] * inv, c[2][0] * inv],
        [c[0][1] * inv, c[1][1] * inv, c[2][1] * inv],
        [c[0][2] * inv, c[1][2] * inv, c[2][2] * inv],
    ]
}

/// Luma matrix coefficients for a coded YCbCr signalling index (ITU-T H.273).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatrixCoeff {
    /// Identity (GBR), no luma.
    Identity,
    Bt709,
    /// ITU-R BT.601 / BT.470 / FCC: kr 0.299, kb 0.114.
    Bt601,
    Smpte240,
    YCgCo,
    /// BT.2020 non-constant-luminance.
    Bt2020Ncl,
    /// BT.2020 constant-luminance.
    Bt2020Cl,
    Smpte2085,
    ChromaDerivedNcl,
    ChromaDerivedCl,
    /// BT.2100 ICtCp.
    ICtCp,
    /// SMPTE ST 2128 IPT-C2.
    IptC2,
    /// Reversible YCgCo-R.
    YCgCoRe,
    /// Reversible-with-overflow YCgCo-Ro.
    YCgCoRo,
}

impl MatrixCoeff {
    /// Map an H.273 / AV1 `matrix_coefficients` code.
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Identity,
            1 => Self::Bt709,
            4 | 5 | 6 => Self::Bt601,
            7 => Self::Smpte240,
            8 => Self::YCgCo,
            9 => Self::Bt2020Ncl,
            10 => Self::Bt2020Cl,
            11 => Self::Smpte2085,
            12 => Self::ChromaDerivedNcl,
            13 => Self::ChromaDerivedCl,
            14 => Self::ICtCp,
            15 => Self::IptC2,
            16 => Self::YCgCoRe,
            17 => Self::YCgCoRo,
            2 | 3 | 18..=u8::MAX => return None,
        })
    }

    /// The code that signals this matrix.
    pub fn code(self) -> u8 {
        match self {
            Self::Identity => 0,
            Self::Bt709 => 1,
            Self::Bt601 => 6,
            Self::Smpte240 => 7,
            Self::YCgCo => 8,
            Self::Bt2020Ncl => 9,
            Self::Bt2020Cl => 10,
            Self::Smpte2085 => 11,
            Self::ChromaDerivedNcl => 12,
            Self::ChromaDerivedCl => 13,
            Self::ICtCp => 14,
            Self::IptC2 => 15,
            Self::YCgCoRe => 16,
            Self::YCgCoRo => 17,
        }
    }

    /// `(kr, kb)` for the luma equation, or `None` when the matrix has no luma.
    pub fn kr_kb(self) -> Option<(f64, f64)> {
        match self {
            Self::Bt709 | Self::Smpte240 => Some((0.2126, 0.0722)),
            Self::Bt601 => Some((0.299, 0.114)),
            Self::Bt2020Ncl | Self::Bt2020Cl => Some((0.2627, 0.0593)),
            Self::Identity
            | Self::YCgCo
            | Self::Smpte2085
            | Self::ChromaDerivedNcl
            | Self::ChromaDerivedCl
            | Self::ICtCp
            | Self::IptC2
            | Self::YCgCoRe
            | Self::YCgCoRo => None,
        }
    }

    /// Constant-luminance matrices need the luma channel restored per channel.
    pub fn constant_luma(self) -> bool {
        matches!(self, Self::Bt2020Cl | Self::ChromaDerivedCl)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Identity => "RGB",
            Self::Bt709 => "BT.709",
            Self::Bt601 => "BT.601",
            Self::Smpte240 => "SMPTE 240M",
            Self::YCgCo | Self::YCgCoRe | Self::YCgCoRo => "YCgCo",
            Self::Bt2020Ncl => "BT.2020 NCL",
            Self::Bt2020Cl => "BT.2020 CL",
            Self::Smpte2085 => "SMPTE 2085",
            Self::ChromaDerivedNcl => "chroma-derived NCL",
            Self::ChromaDerivedCl => "chroma-derived CL",
            Self::ICtCp => "ICtCp",
            Self::IptC2 => "IPT-C2",
        }
    }
}

/// RGB → YCbCr coefficients for one range. Rows are Y, Cb, Cr.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YuvMatrix {
    pub kr: f64,
    pub kb: f64,
    pub full_range: bool,
}

impl YuvMatrix {
    pub fn new(kr: f64, kb: f64, full_range: bool) -> Self {
        Self { kr, kb, full_range }
    }

    pub fn from_matrix(coeff: MatrixCoeff, full_range: bool) -> Option<Self> {
        let (kr, kb) = coeff.kr_kb()?;
        Some(Self::new(kr, kb, full_range))
    }

    pub fn y(self, r: f64, g: f64, b: f64) -> f64 {
        self.kr * r + (1.0 - self.kr - self.kb) * g + self.kb * b
    }

    /// Full-range (0..1) RGB to studio-range or full-range Y', Cb, Cr in 0..1.
    pub fn encode(self, r: f64, g: f64, b: f64) -> [f64; 3] {
        let y = self.y(r, g, b);
        let cb = (b - y) / (2.0 - 2.0 * self.kb);
        let cr = (r - y) / (2.0 - 2.0 * self.kr);
        if self.full_range {
            [y, cb + 0.5, cr + 0.5]
        } else {
            [
                y * (219.0 / 255.0) + 16.0 / 255.0,
                cb * (224.0 / 255.0) + 128.0 / 255.0,
                cr * (224.0 / 255.0) + 128.0 / 255.0,
            ]
        }
    }

    /// Y', Cb, Cr (each 0..1, in the signalled range) back to full-range RGB.
    ///
    /// `Cb`/`Cr` carry the E°_B/E°_R differences, so recovering R and B means
    /// multiplying them back out by 2(1−Kr) and 2(1−Kb).
    pub fn decode(self, y: f64, cb: f64, cr: f64) -> [f64; 3] {
        let (e_y, e_b, e_r) = if self.full_range {
            (y, cb - 0.5, cr - 0.5)
        } else {
            (
                (y * 255.0 - 16.0) / 219.0,
                (cb * 255.0 - 128.0) / 224.0,
                (cr * 255.0 - 128.0) / 224.0,
            )
        };
        let dr = (2.0 - 2.0 * self.kr) * e_r;
        let db = (2.0 - 2.0 * self.kb) * e_b;
        let kg = 1.0 - self.kr - self.kb;
        [
            e_y + dr,
            e_y - (self.kr * dr + self.kb * db) / kg,
            e_y + db,
        ]
    }
}

/// Bradford cone-response transform, used to adapt between white points.
const BRADFORD: [[f64; 3]; 3] = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0634],
];

/// 3×3 linear transform mapping linear RGB of `src` to linear RGB of `dst`.
/// Includes Bradford adaptation between the two white points.
pub fn rgb_to_rgb(src: Primaries, dst: Primaries) -> [[f64; 3]; 3] {
    let adapt = adapt_matrix(src.white, dst.white);
    let m = mul3(adapt, src.rgb_to_xyz());
    mul3(dst.xyz_to_rgb(), m)
}

fn adapt_matrix(from: Chromaticity, to: Chromaticity) -> [[f64; 3]; 3] {
    let cone = |w: Chromaticity| mul3v(BRADFORD, xyz_of(w));
    let s = cone(from);
    let d = cone(to);
    let scale = [
        [d[0] / s[0], 0.0, 0.0],
        [0.0, d[1] / s[1], 0.0],
        [0.0, 0.0, d[2] / s[2]],
    ];
    let inv_bradford = inv3(BRADFORD);
    mul3(inv_bradford, mul3(scale, BRADFORD))
}

fn mul3(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

fn mul3v(a: [[f64; 3]; 3], v: Col) -> Col {
    [
        a[0][0] * v[0] + a[0][1] * v[1] + a[0][2] * v[2],
        a[1][0] * v[0] + a[1][1] * v[1] + a[1][2] * v[2],
        a[2][0] * v[0] + a[2][1] * v[1] + a[2][2] * v[2],
    ]
}

pub fn apply(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn kr_kb_matches_published_luma_weights() {
        // For BT.709 and BT.2020 the standard's luma weights are exactly the
        // chromaticity derivation, so this checks the solver end to end.
        let (kr, kb) = Primaries::BT709.kr_kb();
        assert!(close(kr, 0.2126, 5e-4), "{kr} {kb}");
        assert!(close(kb, 0.0722, 5e-4), "{kr} {kb}");
        let (kr, kb) = Primaries::BT2020.kr_kb();
        assert!(close(kr, 0.2627, 5e-4), "{kr} {kb}");
        assert!(close(kb, 0.0593, 5e-4), "{kr} {kb}");
    }

    #[test]
    fn rgb_to_xyz_maps_white_to_the_illuminant() {
        // The derivation is only right if unitalised RGB reaches the white
        // point's own chromaticity at Y = 1 — true for every set, including
        // the BT.601 ones whose signalled luma weights are not derived.
        for p in [
            Primaries::BT709,
            Primaries::BT2020,
            Primaries::BT601_EBU,
            Primaries::SMPTE170M,
            Primaries::BT470M,
            Primaries::DCI_P3,
            Primaries::DISPLAY_P3,
        ] {
            let xyz = apply(p.rgb_to_xyz(), [1.0, 1.0, 1.0]);
            let w = xyz_of(p.white);
            assert!(close(xyz[1], 1.0, 1e-9), "{p:?} Y = {}", xyz[1]);
            assert!(close(xyz[0] / xyz[1], w[0] / w[1], 1e-9), "{p:?} x");
            assert!(close(xyz[2] / xyz[1], w[2] / w[1], 1e-9), "{p:?} z");
            let back = apply(p.xyz_to_rgb(), xyz);
            for (got, want) in back.iter().zip([1.0, 1.0, 1.0]) {
                assert!(close(*got, want, 1e-9), "{p:?}");
            }
        }
    }

    #[test]
    fn gamut_remap_is_identity_within_one_space() {
        let m = rgb_to_rgb(Primaries::BT709, Primaries::BT709);
        for (got, want) in m.iter().flatten().zip([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]) {
            assert!(close(*got, want, 1e-9), "{got}");
        }
    }

    #[test]
    fn bt709_matrix_rows_match_itu_rec_709() {
        let m = YuvMatrix::new(0.2126, 0.0722, true);
        let [y, cb, cr] = m.encode(1.0, 1.0, 1.0);
        assert!(close(y, 1.0, 1e-12));
        assert!(close(cb, 0.5, 1e-12));
        assert!(close(cr, 0.5, 1e-12));
        let [y, ..] = m.encode(1.0, 0.0, 0.0);
        assert!(close(y, 0.2126, 1e-12));
    }

    #[test]
    fn decode_matrix_matches_the_published_coefficients() {
        // Rec.422/709/601 quote the decode side as additive gains on the
        // E°_B/E°_R differences; these are the numbers a decoder ships with.
        for (kr, kb, r_chroma, b_chroma, g_from_cb, g_from_cr) in [
            (0.299, 0.114, 1.402, 1.772, -0.344_136, -0.714_136),
            (0.2126, 0.0722, 1.5748, 1.8556, -0.1873, -0.4681),
            (0.2627, 0.0593, 1.4746, 1.8814, -0.164_55, -0.570_56),
        ] {
            let m = YuvMatrix::new(kr, kb, true);
            let [r, g, b] = m.decode(0.5, 0.5 + 0.1, 0.5 + 0.2);
            assert!(close(r, 0.5 + r_chroma * 0.2, 1e-3), "{r}");
            assert!(close(b, 0.5 + b_chroma * 0.1, 1e-3), "{b}");
            assert!(close(g, 0.5 + g_from_cb * 0.1 + g_from_cr * 0.2, 1e-3), "{g}");
            let [y, cb, cr] = m.encode(0.5, 0.5, 0.5);
            assert!(close(y, 0.5, 1e-12) && close(cb, 0.5, 1e-12) && close(cr, 0.5, 1e-12));
        }
    }

    #[test]
    fn studio_range_round_trips() {
        let m = YuvMatrix::new(0.2627, 0.0593, false);
        for (r, g, b) in [
            (0.0, 0.0, 0.0),
            (1.0, 1.0, 1.0),
            (0.18, 0.18, 0.18),
            (0.9, 0.11, 0.22),
        ] {
            let [y, cb, cr] = m.encode(r, g, b);
            let [r2, g2, b2] = m.decode(y, cb, cr);
            assert!(close(r, r2, 1e-9), "{r} {g} {b} -> {r2} {g2} {b2}");
            assert!(close(g, g2, 1e-9));
            assert!(close(b, b2, 1e-9));
        }
    }

    #[test]
    fn bt709_to_bt2020_is_identity_on_rec709_gamut() {
        let m = rgb_to_rgb(Primaries::BT709, Primaries::BT2020);
        let mi = rgb_to_rgb(Primaries::BT2020, Primaries::BT709);
        for v in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.3, 0.6, 0.1]] {
            let rt = apply(mi, apply(m, v));
            for i in 0..3 {
                assert!(close(v[i], rt[i], 1e-9), "{v:?} -> {rt:?}");
            }
        }
    }

    #[test]
    fn bt709_gamut_fits_inside_bt2020() {
        let to2020 = rgb_to_rgb(Primaries::BT709, Primaries::BT2020);
        for v in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            let out = apply(to2020, v);
            assert!(
                out.iter().all(|c| *c > -1e-9 && *c < 1.2),
                "{v:?} -> {out:?}"
            );
        }
        // BT.2020's green sits outside BT.709, so the reverse needs a negative leg.
        let to709 = rgb_to_rgb(Primaries::BT2020, Primaries::BT709);
        let green = apply(to709, [0.0, 1.0, 0.0]);
        assert!(green[0] < 0.0 && green[2] < 0.0, "{green:?}");
    }

    #[test]
    fn white_point_maps_to_itself() {
        let m = rgb_to_rgb(Primaries::SMPTE170M, Primaries::BT709);
        let w = apply(m, [1.0, 1.0, 1.0]);
        for v in w {
            assert!(close(v, 1.0, 1e-9), "{w:?}");
        }
    }

    #[test]
    fn matrix_codes_round_trip() {
        for code in 0u8..=17 {
            let Some(coeff) = MatrixCoeff::from_code(code) else {
                assert!(matches!(code, 2 | 3), "unexpected gap at {code}");
                continue;
            };
            assert_eq!(
                MatrixCoeff::from_code(coeff.code()),
                Some(coeff),
                "{code} -> {:?}",
                coeff.label()
            );
            assert!(!coeff.label().is_empty());
        }
        assert_eq!(MatrixCoeff::from_code(18), None);
        assert_eq!(MatrixCoeff::from_code(255), None);
        assert!(MatrixCoeff::Bt2020Cl.constant_luma());
        assert!(!MatrixCoeff::Bt2020Ncl.constant_luma());
        assert_eq!(MatrixCoeff::YCgCo.kr_kb(), None);
    }

    #[test]
    fn primaries_codes_cover_h273() {
        for (code, want_kr) in [(1u8, 0.2126f64), (9, 0.2627)] {
            let p = Primaries::from_code(code).expect("known primaries");
            let (kr, _) = p.kr_kb();
            assert!((kr - want_kr).abs() < 5e-4, "{code}: {kr}");
            assert_eq!(p.code(), Some(code));
        }
        assert!(Primaries::from_code(2).is_none());
        assert!(Primaries::from_code(8).is_none());
    }
}
