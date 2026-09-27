//! 1D and 3D colour lookup tables: `.cube` / `.3dl` parsing, sampling, inversion.

use crate::color::log::Log;
use crate::color::primaries::{apply, rgb_to_rgb, Primaries};
use crate::color::tonemap::{compress_gamut, tone_map_rgb, ContentLight, DisplayTarget, ToneMap};
use crate::color::transfer::{hlg_ootf_rgb, hlg_system_gamma, Transfer};
use crate::invalid;
use crate::Result;

/// How to interpolate between grid nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Interpolation {
    Nearest,
    #[default]
    Trilinear,
    /// Six-tetrahedron split; sharper edges, no ringing on graded ramps.
    Tetrahedral,
}

/// A per-channel 1D LUT.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut1d {
    /// `data[channel][code]`, each normalised 0..=1.
    pub data: [Vec<f32>; 3],
}

impl Lut1d {
    pub fn len(&self) -> usize {
        self.data[0].len()
    }

    pub fn is_empty(&self) -> bool {
        self.data[0].is_empty()
    }

    pub fn identity(size: usize) -> Self {
        let size = size.max(2);
        let ramp: Vec<f32> = (0..size).map(|i| i as f32 / (size - 1) as f32).collect();
        Self {
            data: [ramp.clone(), ramp.clone(), ramp],
        }
    }

    pub fn sample(&self, channel: usize, value: f32) -> f32 {
        let lut = &self.data[channel.min(2)];
        if lut.is_empty() {
            return value;
        }
        if lut.len() == 1 {
            return lut[0];
        }
        let pos = value.clamp(0.0, 1.0) * (lut.len() - 1) as f32;
        let lo = pos.floor() as usize;
        let hi = (lo + 1).min(lut.len() - 1);
        let frac = pos - lo as f32;
        lut[lo].mul_add(1.0 - frac, lut[hi] * frac)
    }
}

/// A 3D colour cube plus the input domain it was authored for.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut3d {
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// Node values with red varying fastest: `data[r + size·(g + size·b)]`.
    pub data: Vec<[f32; 3]>,
}

impl Lut3d {
    pub fn identity(size: usize) -> Self {
        let size = size.clamp(2, 128);
        let mut data = Vec::with_capacity(size * size * size);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    data.push([
                        r as f32 / (size - 1) as f32,
                        g as f32 / (size - 1) as f32,
                        b as f32 / (size - 1) as f32,
                    ]);
                }
            }
        }
        Self {
            size,
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            data,
        }
    }

    /// Build a cube by evaluating a colour transform on an `size³` grid.
    pub fn from_fn<F: FnMut([f32; 3]) -> [f32; 3]>(size: usize, mut f: F) -> Self {
        let mut lut = Self::identity(size);
        for v in lut.data.iter_mut() {
            *v = f(*v);
        }
        lut
    }

    fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        self.data[r + self.size * (g + self.size * b)]
    }

    /// Normalise an input into the authored domain, clamping outside it.
    fn scale(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for i in 0..3 {
            let lo = self.domain_min[i];
            let span = (self.domain_max[i] - lo).max(1e-6);
            out[i] = ((rgb[i] - lo) / span).clamp(0.0, 1.0);
        }
        out
    }

    pub fn sample(&self, rgb: [f32; 3], interp: Interpolation) -> [f32; 3] {
        let n = self.size - 1;
        let [r, g, b] = self.scale(rgb);
        let (fr, fg, fb) = (r * n as f32, g * n as f32, b * n as f32);
        let (r0, g0, b0) = (fr as usize, fg as usize, fb as usize);
        let (dr, dg, db) = (fr - r0 as f32, fg - g0 as f32, fb - b0 as f32);
        match interp {
            Interpolation::Nearest => {
                let ri = if dr > 0.5 { (r0 + 1).min(n) } else { r0 };
                let gi = if dg > 0.5 { (g0 + 1).min(n) } else { g0 };
                let bi = if db > 0.5 { (b0 + 1).min(n) } else { b0 };
                self.at(ri, gi, bi)
            }
            Interpolation::Trilinear => {
                let (r1, g1, b1) = ((r0 + 1).min(n), (g0 + 1).min(n), (b0 + 1).min(n));
                let c000 = self.at(r0, g0, b0);
                let c100 = self.at(r1, g0, b0);
                let c010 = self.at(r0, g1, b0);
                let c110 = self.at(r1, g1, b0);
                let c001 = self.at(r0, g0, b1);
                let c101 = self.at(r1, g0, b1);
                let c011 = self.at(r0, g1, b1);
                let c111 = self.at(r1, g1, b1);
                let mut out = [0.0f32; 3];
                for ch in 0..3 {
                    let x00 = c000[ch].mul_add(1.0 - dr, c100[ch] * dr);
                    let x10 = c010[ch].mul_add(1.0 - dr, c110[ch] * dr);
                    let x01 = c001[ch].mul_add(1.0 - dr, c101[ch] * dr);
                    let x11 = c011[ch].mul_add(1.0 - dr, c111[ch] * dr);
                    let y0 = x00.mul_add(1.0 - dg, x10 * dg);
                    let y1 = x01.mul_add(1.0 - dg, x11 * dg);
                    out[ch] = y0.mul_add(1.0 - db, y1 * db);
                }
                out
            }
            Interpolation::Tetrahedral => {
                // Order the axes by offset, then walk the four vertices of that
                // tetrahedron: origin, +biggest, +middle, +all.
                let d = [dr, dg, db];
                let mut order = [0usize, 1, 2];
                order.sort_by(|x, y| d[*y].total_cmp(&d[*x]));
                let node = |mask: usize| -> [f32; 3] {
                    self.at(
                        (r0 + (mask & 1)).min(n),
                        (g0 + ((mask >> 1) & 1)).min(n),
                        (b0 + ((mask >> 2) & 1)).min(n),
                    )
                };
                let m1 = 1 << order[0];
                let m2 = m1 | (1 << order[1]);
                // The four weights are the ordered offsets' differences, so the
                // sum telescopes to 1 and each vertex only sees its own tetra.
                let (s0, s1, s2) = (d[order[0]], d[order[1]], d[order[2]]);
                let n0 = node(0);
                let n1 = node(m1);
                let n2 = node(m2);
                let n3 = node(7);
                let mut out = [0.0f32; 3];
                for ch in 0..3 {
                    out[ch] = n0[ch].mul_add(
                        1.0 - s0,
                        n1[ch] * (s0 - s1) + n2[ch] * (s1 - s2) + n3[ch] * s2,
                    );
                }
                out
            }
        }
    }
}

/// A parsed LUT file.
#[derive(Clone, Debug, PartialEq)]
pub enum Lut {
    One(Lut1d),
    Three(Lut3d),
}

impl Lut {
    /// Resolve/Adobe `.cube`.
    pub fn from_cube(text: &str) -> Result<Self> {
        let mut one_size = None;
        let mut three_size = None;
        let mut domain_min = [0.0f32; 3];
        let mut domain_max = [1.0f32; 3];
        let mut values: Vec<f32> = Vec::new();
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let upper = line.to_ascii_uppercase();
            if upper.starts_with("TITLE") {
                continue;
            }
            if let Some(rest) = upper.strip_prefix("LUT_1D_SIZE") {
                if three_size.is_some() {
                    return Err(invalid("a .cube cannot declare both 1D and 3D size"));
                }
                one_size = Some(parse_size(rest, line_no)?);
                continue;
            }
            if let Some(rest) = upper.strip_prefix("LUT_3D_SIZE") {
                if one_size.is_some() {
                    return Err(invalid("a .cube cannot declare both 1D and 3D size"));
                }
                three_size = Some(parse_size(rest, line_no)?);
                continue;
            }
            if let Some(rest) = upper.strip_prefix("DOMAIN_MIN") {
                domain_min = parse3(rest, line_no)?;
                continue;
            }
            if let Some(rest) = upper.strip_prefix("DOMAIN_MAX") {
                domain_max = parse3(rest, line_no)?;
                continue;
            }
            if upper.starts_with("LUT_1D_INPUT_RANGE")
                || upper.starts_with("LUT_1D_OUTPUT_RANGE")
                || upper.starts_with("OUTPUT_DOMAIN_MIN")
                || upper.starts_with("OUTPUT_DOMAIN_MAX")
            {
                continue;
            }
            if is_number_line(line) {
                values.extend(
                    line.split_whitespace()
                        .filter_map(|t| t.parse::<f32>().ok()),
                );
                continue;
            }
            return Err(invalid(&format!(
                "unknown .cube key on line {}",
                line_no + 1
            )));
        }
        let nan = values.iter().any(|v| !v.is_finite());
        if nan {
            return Err(invalid("a .cube value is not a number"));
        }
        match (one_size, three_size) {
            (Some(n), None) => {
                let rows = if values.len() == n * 3 {
                    3
                } else if values.len() == n {
                    1
                } else {
                    return Err(invalid(&format!(
                        "1D LUT expected {n} or {} values, got {}",
                        n * 3,
                        values.len()
                    )));
                };
                let mut data = [
                    Vec::with_capacity(n),
                    Vec::with_capacity(n),
                    Vec::with_capacity(n),
                ];
                for chunk in values.chunks(rows) {
                    for ch in 0..3 {
                        let v = if rows == 1 { chunk[0] } else { chunk[ch] };
                        let lo = domain_min[ch];
                        let span = (domain_max[ch] - lo).max(1e-6);
                        data[ch].push(((v - lo) / span).clamp(0.0, 1.0));
                    }
                }
                Ok(Self::One(Lut1d { data }))
            }
            (None, Some(n)) => {
                if values.len() != n * n * n * 3 {
                    return Err(invalid(&format!(
                        "3D LUT expected {} values, got {}",
                        n * n * n * 3,
                        values.len()
                    )));
                }
                let data = values
                    .chunks_exact(3)
                    .map(|c| {
                        [
                            c[0].clamp(0.0, 1.0),
                            c[1].clamp(0.0, 1.0),
                            c[2].clamp(0.0, 1.0),
                        ]
                    })
                    .collect();
                Ok(Self::Three(Lut3d {
                    size: n,
                    domain_min,
                    domain_max,
                    data,
                }))
            }
            (None, None) => Err(invalid("a .cube must declare LUT_1D_SIZE or LUT_3D_SIZE")),
            (Some(_), Some(_)) => Err(invalid("a .cube cannot declare both sizes")),
        }
    }

    /// Autodesk/Avid `.3dl`: a size line then 12-bit integer rows.
    pub fn from_3dl(text: &str) -> Result<Self> {
        let mut rows: Vec<[f32; 3]> = Vec::new();
        let mut size = None;
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() == 1 {
                let n: usize = fields[0]
                    .parse()
                    .map_err(|_| invalid(&format!("bad .3dl size on line {}", line_no + 1)))?;
                if !(2..=64).contains(&n) {
                    return Err(invalid("a .3dl size must be 2..=64"));
                }
                if size.is_some() {
                    return Err(invalid("a .3dl declares a size more than once"));
                }
                size = Some(n);
                continue;
            }
            if fields.len() > 1 && fields[0].parse::<f32>().is_err() {
                return Err(invalid(&format!(
                    "unknown .3dl key on line {}",
                    line_no + 1
                )));
            }
            if fields.len() < 3 {
                return Err(invalid(&format!(
                    "a .3dl row needs 3 channels on line {}",
                    line_no + 1
                )));
            }
            let mut v = [0.0f32; 3];
            for (i, f) in fields.iter().take(3).enumerate() {
                let n: f32 = f
                    .parse()
                    .map_err(|_| invalid(&format!("bad .3dl value `{f}`")))?;
                v[i] = (n / 4095.0).clamp(0.0, 1.0);
            }
            rows.push(v);
        }
        let size = size.ok_or_else(|| invalid("a .3dl has no size line"))?;
        if rows.len() != size * size * size {
            return Err(invalid(&format!(
                "a .3dl expected {} rows, got {}",
                size * size * size,
                rows.len()
            )));
        }
        Ok(Self::Three(Lut3d {
            size,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            data: rows,
        }))
    }

    /// Read either format from content: a `.cube` always names its size.
    pub fn from_text(text: &str) -> Result<Self> {
        if text.contains("_SIZE") {
            Self::from_cube(text)
        } else {
            Self::from_3dl(text)
        }
    }

    pub fn sample(&self, rgb: [f32; 3], interp: Interpolation) -> [f32; 3] {
        match self {
            Self::One(l) => [
                l.sample(0, rgb[0]),
                l.sample(1, rgb[1]),
                l.sample(2, rgb[2]),
            ],
            Self::Three(l) => l.sample(rgb, interp),
        }
    }

    /// Grid side length, used by the OSD.
    pub fn size(&self) -> usize {
        match self {
            Self::One(l) => l.len(),
            Self::Three(l) => l.size,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::One(_) => "1D",
            Self::Three(_) => "3D",
        }
    }

    /// True when sampling changes nothing within `tol`.
    pub fn is_identity(&self, tol: f32) -> bool {
        (0..=16).all(|i| {
            let v = i as f32 / 16.0;
            let out = self.sample([v, v, v], Interpolation::Tetrahedral);
            out.iter().zip([v, v, v]).all(|(a, b)| (a - b).abs() <= tol)
        })
    }

    /// Serialise back to `.cube`, so a generated LUT survives a write/read cycle.
    pub fn to_cube(&self) -> String {
        let mut out = String::new();
        match self {
            Self::One(l) => {
                out.push_str(&format!("LUT_1D_SIZE {}\n", l.len()));
                for i in 0..l.len() {
                    out.push_str(&format!(
                        "{} {} {}\n",
                        l.data[0][i], l.data[1][i], l.data[2][i]
                    ));
                }
            }
            Self::Three(l) => {
                out.push_str(&format!("LUT_3D_SIZE {}\n", l.size));
                if l.domain_min != [0.0; 3] || l.domain_max != [1.0; 3] {
                    out.push_str(&format!(
                        "DOMAIN_MIN {} {} {}\n",
                        l.domain_min[0], l.domain_min[1], l.domain_min[2]
                    ));
                    out.push_str(&format!(
                        "DOMAIN_MAX {} {} {}\n",
                        l.domain_max[0], l.domain_max[1], l.domain_max[2]
                    ));
                }
                for v in &l.data {
                    out.push_str(&format!("{} {} {}\n", v[0], v[1], v[2]));
                }
            }
        }
        out
    }
}

fn parse_size(rest: &str, line_no: usize) -> Result<usize> {
    let n: usize = rest
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .parse()
        .map_err(|_| invalid(&format!("bad LUT size on line {}", line_no + 1)))?;
    if !(2..=128).contains(&n) {
        return Err(invalid("LUT size must be 2..=128"));
    }
    Ok(n)
}

fn parse3(rest: &str, line_no: usize) -> Result<[f32; 3]> {
    let nums: Vec<f32> = rest
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    if nums.len() != 3 {
        return Err(invalid(&format!(
            "expected 3 numbers on line {}",
            line_no + 1
        )));
    }
    Ok([nums[0], nums[1], nums[2]])
}

fn is_number_line(line: &str) -> bool {
    line.split_whitespace()
        .next()
        .is_some_and(|t| t.parse::<f32>().is_ok())
}

/// Sample a transfer function as a 1D LUT over `n` codes.
pub fn transfer_lut(transfer: Transfer, n: usize) -> Lut1d {
    let ramp: Vec<f32> = (0..n)
        .map(|i| {
            let v = i as f32 / (n - 1) as f32;
            transfer.eotf(v).unwrap_or(v)
        })
        .collect();
    Lut1d {
        data: [ramp.clone(), ramp.clone(), ramp],
    }
}

/// Recipe for a generated grading cube.
///
/// This is what a camera vendor's "identity" LUT or an HDR→SDR conversion LUT
/// is built from: decode the source signal, adapt the gamut, optionally
/// compress the highlights for one display, then re-encode for the destination.
#[derive(Clone, Copy, Debug)]
pub struct CubePlan {
    /// Transfer the input codes are read with.
    pub from: Transfer,
    /// Camera log curve the input codes carry, which replaces `from`.
    pub log: Option<Log>,
    /// Primaries the input codes are stated in.
    pub source: Primaries,
    /// Transfer the output codes are written with.
    pub to: Transfer,
    /// Primaries the output codes are stated in.
    pub dest: Primaries,
    /// Grid edge length, 2..=128.
    pub size: usize,
    /// Reference white for signals with no absolute scale, in cd/m².
    pub sdr_white_nits: f32,
    /// The destination panel, when highlights are to be compressed.
    pub target: Option<DisplayTarget>,
    /// The source's own peak light, when highlights are to be compressed.
    pub content: ContentLight,
    /// Curve used when compressing; `None` maps light 1:1.
    pub tone_map: Option<ToneMap>,
}

impl CubePlan {
    /// A pure transfer-and-gamut converter, with no highlight compression.
    pub fn transfer(
        from: Transfer,
        to: Transfer,
        source: Primaries,
        dest: Primaries,
        size: usize,
    ) -> Self {
        Self {
            from,
            log: None,
            source,
            to,
            dest,
            size,
            sdr_white_nits: crate::color::transfer::SDR_PEAK_NITS,
            target: None,
            content: ContentLight::default(),
            tone_map: None,
        }
    }

    /// Camera log material read as `profile`, converted to `to`/`dest`.
    ///
    /// `source` is the camera gamut the profile was authored with; passing the
    /// destination gamut makes the cube a pure transfer conversion.
    pub fn camera_log(
        profile: Log,
        to: Transfer,
        source: Primaries,
        dest: Primaries,
        size: usize,
    ) -> Self {
        Self {
            log: Some(profile),
            ..Self::transfer(Transfer::Linear, to, source, dest, size)
        }
    }

    /// Camera log material converted with the gamut its vendor publishes.
    ///
    /// Profiles whose working gamut has no citable chromaticities convert as
    /// though the material is already in `dest`, which leaves the curve
    /// correction intact and the colour untouched.
    pub fn camera(profile: Log, to: Transfer, dest: Primaries, size: usize) -> Self {
        let source = profile.gamut().unwrap_or(dest);
        Self::camera_log(profile, to, source, dest, size)
    }

    /// The same converter, tone mapped for one display.
    pub fn grade(
        mut self,
        tone_map: ToneMap,
        target: DisplayTarget,
        content: ContentLight,
    ) -> Self {
        self.tone_map = Some(tone_map);
        self.target = Some(target);
        self.content = content;
        self
    }

    /// Build the cube by evaluating the pipeline at every node.
    pub fn build(&self) -> Lut3d {
        let m = rgb_to_rgb(self.source, self.dest);
        let (kr, kb) = self.dest.kr_kb();
        let (kr, kb) = (kr as f32, kb as f32);
        // The panel the plan is baked for, in cd/m². An unmapped plan has no
        // panel of its own; BT.2100's reference display answers for it.
        let panel = self.target.map_or(1_000.0, |t| t.peak_nits);
        // A log curve states 1.0 at diffuse white, while an SDR video curve has
        // no absolute scale at all and PQ already is one. HLG is the one HDR
        // curve whose 1.0 means "whatever this panel reaches": BT.2100 makes its
        // scene light display-size dependent and lets γ follow the panel, so
        // fixing HLG to the 1 000 cd/m² reference here would claim ten times the
        // headroom a 100-nit destination has and clip every code above 0.45 to
        // white. PQ keeps its absolute scale, because it states one.
        let scale = match self.log {
            Some(_) => self.sdr_white_nits,
            None if self.from == Transfer::Hlg => panel,
            None => self.from.full_scale_nits(self.sdr_white_nits),
        };
        Lut3d::from_fn(self.size, |rgb| {
            let mut lin = [0.0f32; 3];
            for (out, v) in lin.iter_mut().zip(rgb) {
                *out = match self.log {
                    Some(profile) => profile.to_linear(v),
                    None => self.from.eotf(v).unwrap_or(v),
                };
            }
            if self.from == Transfer::Hlg && self.log.is_none() {
                lin = hlg_ootf_rgb(lin, kr, kb, hlg_system_gamma(panel));
            }
            let mapped = compress_gamut(apply(m, lin.map(f64::from)).map(|v| v as f32), self.dest);
            let display = match self.tone_map {
                None => mapped,
                Some(mode) => tone_map_rgb(
                    mapped.map(|v| v * scale),
                    mode,
                    self.target.unwrap_or_default(),
                    self.content,
                    self.dest,
                ),
            };
            let mut out = [0.0f32; 3];
            for (o, v) in out.iter_mut().zip(display) {
                *o = self.to.oetf(v).unwrap_or(v).clamp(0.0, 1.0);
            }
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    const CUBE_2: &str = "\
# a two-node cube
TITLE \"tiny\"
LUT_3D_SIZE 2
0 0 0
1 0 0
0 1 0
1 1 0
0 0 1
1 0 1
0 1 1
1 1 1
";

    #[test]
    fn cube_3d_parses_with_red_varying_fastest() {
        let lut = Lut::from_cube(CUBE_2).unwrap();
        assert_eq!(lut.size(), 2);
        let Lut::Three(l) = &lut else {
            panic!("expected a 3D LUT");
        };
        assert_eq!(l.data[1], [1.0, 0.0, 0.0]);
        assert_eq!(l.data[2], [0.0, 1.0, 0.0]);
        assert_eq!(l.data[4], [0.0, 0.0, 1.0]);
        assert!(lut.is_identity(1e-5));
    }

    #[test]
    fn cube_1d_accepts_one_or_three_channels_per_row() {
        let one = Lut::from_cube("LUT_1D_SIZE 2\n0\n1\n0\n1\n0\n1\n").unwrap();
        assert_eq!(one.size(), 2);
        assert!(matches!(one, Lut::One(_)));
        let three = Lut::from_cube("LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
        assert_eq!(
            three.sample([0.5, 0.5, 0.5], Interpolation::default()),
            [0.5, 0.5, 0.5]
        );
    }

    #[test]
    fn malformed_cubes_are_rejected() {
        for bad in [
            "",
            "LUT_3D_SIZE 2\n0 0 0\n",
            "LUT_1D_SIZE 4\n0 0 0\n1 1 1\n",
            "LUT_3D_SIZE 2\nLUT_1D_SIZE 2\n0 0 0\n",
            "BOGUS 3\n",
            "LUT_3D_SIZE 0\n",
            "LUT_3D_SIZE abc\n",
        ] {
            assert!(Lut::from_cube(bad).is_err(), "accepted: {bad:?}");
        }
    }

    #[test]
    fn domain_is_applied_on_read_and_write() {
        let text = "LUT_3D_SIZE 2\nDOMAIN_MIN 0.05 0.05 0.05\nDOMAIN_MAX 0.95 0.95 0.95\n\
                    0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let lut = Lut::from_cube(text).unwrap();
        let Lut::Three(l) = &lut else {
            panic!("3d");
        };
        assert_eq!(l.domain_min, [0.05; 3]);
        // Below the domain clamps to the first node.
        assert_eq!(
            l.sample([0.0, 0.5, 0.5], Interpolation::Nearest),
            [0.0, 0.0, 0.0]
        );
    }

    #[test]
    fn cube_round_trips_through_text() {
        let src = Lut3d::identity(4);
        let text = Lut::Three(src.clone()).to_cube();
        let back = Lut::from_cube(&text).unwrap();
        assert_eq!(back, Lut::Three(src));
    }

    #[test]
    fn every_interpolator_hits_the_nodes_exactly() {
        // Each node's value is authored at that node's own coordinate, so
        // sampling there must return it under every interpolator. The grid is
        // 3³ so node coordinates are exact binary fractions.
        let f = |v: [f32; 3]| {
            [
                v[0].mul_add(0.75, 0.1),
                v[1].mul_add(0.5, v[0] * 0.2),
                1.0 - v[2].mul_add(0.4, v[1] * 0.1),
            ]
        };
        let lut = Lut3d::from_fn(3, f);
        assert_eq!(lut.data.len(), 27);
        let n = (lut.size - 1) as f32;
        for b in 0..=2usize {
            for g in 0..=2 {
                for r in 0..=2 {
                    let p = [r as f32 / n, g as f32 / n, b as f32 / n];
                    let want = f(p);
                    for mode in [
                        Interpolation::Nearest,
                        Interpolation::Trilinear,
                        Interpolation::Tetrahedral,
                    ] {
                        let got = lut.sample(p, mode);
                        for ch in 0..3 {
                            assert!(
                                close(got[ch], want[ch], 1e-6),
                                "{mode:?} node {r},{g},{b} ch {ch}: {got:?} vs {want:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn affine_lut_is_reproduced_everywhere_by_trilinear_and_tetrahedral() {
        // Both interpolators are exact for a function that is linear in each
        // axis, so any error here is a wrong weight rather than quantisation.
        let f = |v: [f32; 3]| {
            [
                v[0] * 0.5 + v[1] * 0.25 + v[2] * 0.25,
                0.1 + v[0] * 0.2 - v[1] * 0.05 + v[2] * 0.75,
                v[0] * 0.9 - v[1] * 0.6 + v[2] * 0.7,
            ]
        };
        let lut = Lut3d::from_fn(5, f);
        for i in 1..=12u32 {
            // Every probe has to stay inside the authored domain; outside it
            // the sample is legitimately clamped, not affine-exact.
            let p = [
                i as f32 / 13.0,
                (13 - i) as f32 / 13.0,
                (i * i) as f32 / 169.0,
            ];
            let want = f(p);
            for mode in [Interpolation::Trilinear, Interpolation::Tetrahedral] {
                let got = lut.sample(p, mode);
                for ch in 0..3 {
                    assert!(
                        close(got[ch], want[ch], 1e-5),
                        "{mode:?} {p:?} ch {ch}: {got:?} vs {want:?}"
                    );
                }
            }
            assert_ne!(lut.sample(p, Interpolation::Nearest), want);
        }
    }

    #[test]
    fn both_interpolators_converge_on_the_sampled_function() {
        let f = |v: [f32; 3]| {
            [
                v[0].mul_add(v[0], 0.2 * v[1]),
                v[1] * v[2] * 0.9 + 0.05,
                1.0 - v[2].mul_add(v[2], 0.0),
            ]
        };
        let probe = [0.37f32, 0.61, 0.19];
        let want = f(probe);
        let grid_err = |size: usize, mode: Interpolation| {
            let got = Lut3d::from_fn(size, f).sample(probe, mode);
            (0..3)
                .map(|ch| (got[ch] - want[ch]).abs())
                .fold(0.0f32, f32::max)
        };
        let coarse_tri = grid_err(5, Interpolation::Trilinear);
        let fine_tri = grid_err(33, Interpolation::Trilinear);
        let coarse_tet = grid_err(5, Interpolation::Tetrahedral);
        let fine_tet = grid_err(33, Interpolation::Tetrahedral);
        assert!(coarse_tri < 0.02, "trilinear grid error {coarse_tri}");
        assert!(coarse_tet < 0.02, "tetrahedral grid error {coarse_tet}");
        assert!(
            fine_tri * 4.0 < coarse_tri,
            "trilinear {coarse_tri} -> {fine_tri}"
        );
        assert!(
            fine_tet * 4.0 < coarse_tet,
            "tetrahedral {coarse_tet} -> {fine_tet}"
        );
    }

    #[test]
    fn camera_log_cube_delivers_the_destination_curve_at_the_vendor_anchor() {
        // S-Log3 at Sony's own 18 % code (420/1023) has to arrive at Rec.709's
        // code for 18 % reflectance when nothing but the transfer changes.
        let plan = CubePlan::camera_log(
            Log::SLog3,
            Transfer::Bt709,
            Primaries::BT709,
            Primaries::BT709,
            65,
        );
        let cube = plan.build();
        let grey = cube.sample([420.0 / 1023.0; 3], Interpolation::Trilinear);
        assert!(close(grey[0], 0.409_008, 2e-3), "{grey:?}");
        // Sub-black exposure is clamped away, and the top of a log curve is far
        // above white, so it has to saturate the destination.
        let black = cube.sample([0.0; 3], Interpolation::Trilinear);
        assert!(close(black[0], 0.0, 1e-3), "{black:?}");
        let top = cube.sample([1.0; 3], Interpolation::Trilinear);
        assert!(close(top[0], 1.0, 1e-3), "{top:?}");
    }

    #[test]
    fn camera_cube_converts_through_the_vendor_gamut_when_one_is_published() {
        let wide = CubePlan::camera(Log::SLog3, Transfer::Bt709, Primaries::BT709, 65).build();
        let plain = CubePlan::camera_log(
            Log::SLog3,
            Transfer::Bt709,
            Primaries::BT709,
            Primaries::BT709,
            65,
        )
        .build();
        // Sony's 90 % code on a pure S-Gamut3 red is far outside BT.709, so the
        // gamut conversion overshoots 1.0 and the destination clips it, while
        // the same code stated in BT.709 stays below paper white.
        let red = [598.0 / 1023.0, 0.0, 0.0];
        let from_wide = wide.sample(red, Interpolation::Trilinear);
        let from_709 = plain.sample(red, Interpolation::Trilinear);
        assert!(
            from_wide[0] > from_709[0] + 0.02,
            "{from_wide:?} vs {from_709:?}"
        );
        assert!(
            from_wide.iter().all(|v| *v >= 0.0 && *v <= 1.0),
            "{from_wide:?}"
        );
        // The out-of-gamut green and blue legs have to come back up from
        // negative, which is what keeps the clipped red from turning black.
        assert!(from_wide[1] >= 0.0 && from_wide[2] > 0.0, "{from_wide:?}");
        // Every gamut matrix has grey as a fixed point. The residual here is
        // the enclosing cell's off-diagonal corners, which do differ, not the
        // grey nodes themselves.
        let grey = [420.0 / 1023.0; 3];
        let gw = wide.sample(grey, Interpolation::Trilinear);
        let gp = plain.sample(grey, Interpolation::Trilinear);
        for i in 0..3 {
            assert!(close(gw[i], gp[i], 2e-3), "{gw:?} vs {gp:?}");
        }
    }

    #[test]
    fn log_without_a_published_gamut_converts_in_the_destination_space() {
        // Canon draws Cinema Gamut rather than stating it, so guessing its
        // corners would move colour with no source behind the move.
        let plan = CubePlan::camera(Log::CLog3, Transfer::Bt709, Primaries::BT709, 17);
        assert_eq!(plan.source, Primaries::BT709);
        assert!(Log::SLog3.gamut().is_some());
    }

    #[test]
    fn every_log_profile_produces_a_usable_rec709_cube() {
        for profile in Log::ALL {
            let plan = CubePlan::camera_log(
                profile,
                Transfer::Bt709,
                Primaries::BT709,
                Primaries::BT709,
                33,
            );
            let cube = plan.build();
            let mut prev = -1.0f32;
            for i in 0..=32u32 {
                let code = i as f32 / 32.0;
                let got = cube.sample([code; 3], Interpolation::Trilinear)[0];
                assert!(got >= prev - 1e-3, "{profile:?} {code} {prev} -> {got}");
                assert!((0.0..=1.0).contains(&got), "{profile:?} {code} {got}");
                prev = got;
            }
            assert!(prev > 0.9, "{profile:?} never reaches the top");
        }
    }

    #[test]
    fn cube_3dl_parses_12bit_rows() {
        let mut text = String::from("3\n");
        for b in 0..3 {
            for g in 0..3 {
                for r in 0..3 {
                    text.push_str(&format!("{} {} {}\n", r * 2047, g * 2047, b * 2047));
                }
            }
        }
        let lut = Lut::from_3dl(&text).unwrap();
        assert_eq!(lut.size(), 3);
        let out = lut.sample([0.0, 0.0, 0.0], Interpolation::Nearest);
        assert!(close(out[0], 0.0, 1e-6), "{out:?}");
        let hi = lut.sample([1.0, 1.0, 1.0], Interpolation::Nearest);
        assert!(close(hi[0], 1.0, 1e-3), "{hi:?}");
    }

    #[test]
    fn bad_3dl_files_are_rejected() {
        assert!(Lut::from_3dl("3\n0 0 0\n").is_err());
        assert!(Lut::from_3dl("0 0 0\n1 1 1\n").is_err());
        assert!(Lut::from_3dl("3\n0 0\n").is_err());
    }

    #[test]
    fn transfer_lut_is_the_curve_sampled_on_a_code_grid() {
        let lut = transfer_lut(Transfer::Pq, 1024);
        assert_eq!(lut.len(), 1024);
        for (i, v) in lut.data[0].iter().enumerate() {
            let code = i as f32 / 1023.0;
            assert!(close(*v, Transfer::Pq.eotf(code).unwrap(), 1e-7), "{i}");
        }
        assert!(close(lut.data[0][0], 0.0, 1e-9));
        assert!(close(lut.data[0][1023], 1.0, 1e-6));
        // PQ spends codes on highlights: the midpoint of the scale is only
        // ~93 cd/m², which is what makes a naive 8-bit LUT useless for HDR.
        assert!(
            close(lut.data[0][512], 0.009_27, 1e-4),
            "{}",
            lut.data[0][512]
        );
    }

    #[test]
    fn plain_transfer_cube_only_reaches_the_dest_curve() {
        // No tone map: PQ's 10 000-nit scale is carried over as relative light,
        // so 1000 nits (code 0.7518) leaves as 0.1 on the destination scale.
        let cube = CubePlan::transfer(
            Transfer::Pq,
            Transfer::Srgb,
            Primaries::BT2020,
            Primaries::BT709,
            33,
        )
        .build();
        assert_eq!(cube.size, 33);
        let grey = cube.sample([0.751_827_1; 3], Interpolation::Tetrahedral);
        assert!(
            grey.iter().all(|v| close(*v, 0.3480, 3e-3)),
            "1000-nit grey came out {grey:?}"
        );
        assert!(close(
            cube.sample([0.0; 3], Interpolation::Trilinear)[0],
            0.0,
            1e-6
        ));
        assert!(close(
            cube.sample([1.0; 3], Interpolation::Trilinear)[0],
            1.0,
            1e-3
        ));
    }

    #[test]
    fn graded_cube_maps_a_pq_range_onto_an_sdr_display() {
        let cube = CubePlan::transfer(
            Transfer::Pq,
            Transfer::Srgb,
            Primaries::BT2020,
            Primaries::BT709,
            17,
        )
        .grade(
            ToneMap::Reinhard,
            DisplayTarget::sdr(100.0),
            ContentLight {
                max_cll: 1000.0,
                ..Default::default()
            },
        )
        .build();
        let code = |nits: f32| Transfer::Pq.from_nits(nits, 203.0).unwrap();
        let black = cube.sample([code(0.005); 3], Interpolation::Tetrahedral);
        assert!(black[0] < 0.05, "{black:?}");
        // The content peak is the panel's peak by construction.
        let white = cube.sample([code(1000.0); 3], Interpolation::Tetrahedral);
        assert!(close(white[0], 1.0, 1e-3), "{white:?}");
        // 100 cd/m² diffuse white lands mid-scale and stays neutral.
        let grey = cube.sample([code(100.0); 3], Interpolation::Tetrahedral);
        assert!(grey[0] > 0.4 && grey[0] < 0.9, "{grey:?}");
        assert!(
            close(grey[0], grey[1], 1e-4) && close(grey[1], grey[2], 1e-4),
            "{grey:?}"
        );
        // Monotonic along the diagonal, with no banding step at the joint.
        let mut prev = -1.0;
        for i in 0..=100u32 {
            let v = i as f32 / 100.0;
            let out = cube.sample([v; 3], Interpolation::Tetrahedral)[0];
            assert!(out >= prev - 1e-4, "{v}: {prev} -> {out}");
            prev = out;
        }
    }

    /// HLG asks to be read by the panel that shows it: BT.2100-2 normalises its
    /// scene light to that panel's peak and lets γ follow it — 0.78 at 100 cd/m²,
    /// 1.2 at 1 000 — so the same code leaves the grid at a different level for
    /// each, and both follow the standard's own `oetf(inv_oetf(c)^γ)`. What this
    /// rules out is the one reading that cannot be right: claiming the format's
    /// 1 000-cd/m² reference peak *and* compressing onto a 100-nit panel with
    /// nothing stated, which turns every code above 0.45 into pure white.
    #[test]
    fn an_hlg_picture_is_scaled_to_the_panel_that_reads_it() {
        use crate::color::transfer::hlg_inverse_oetf;
        let baked = |peak: f32| {
            CubePlan::transfer(
                Transfer::Hlg,
                Transfer::Bt709,
                Primaries::BT2020,
                Primaries::BT709,
                33,
            )
            .grade(
                ToneMap::Clip,
                DisplayTarget::sdr(peak),
                ContentLight::default(),
            )
            .build()
        };
        for peak in [100.0f32, 1_000.0] {
            let cube = baked(peak);
            let gamma = hlg_system_gamma(peak);
            let want = |c: f32| {
                Transfer::Bt709
                    .oetf(hlg_inverse_oetf(c).powf(gamma))
                    .unwrap()
            };
            // A tenth of the code scale, at both panel sizes.
            for i in 0..=10u32 {
                let c = i as f32 / 10.0;
                let got = cube.sample([c; 3], Interpolation::Tetrahedral)[0];
                assert!(
                    close(got, want(c), 2e-3),
                    "{peak} nits, code {c}: {got} against the standard's {}",
                    want(c)
                );
            }
            assert!(close(
                cube.sample([0.0; 3], Interpolation::Tetrahedral)[0],
                0.0,
                1e-6
            ));
            assert!(close(
                cube.sample([1.0; 3], Interpolation::Tetrahedral)[0],
                1.0,
                1e-3
            ));
            // The four codes a burned-out picture would fold together stay apart,
            // and the mid code is mid-dark rather than white on either panel.
            let steps: Vec<f32> = [0.5f32, 0.65, 0.8, 1.0]
                .iter()
                .map(|c| cube.sample([*c; 3], Interpolation::Tetrahedral)[0])
                .collect();
            assert!(
                steps.windows(2).all(|w| w[1] > w[0] + 5e-2),
                "{peak} nits: {steps:?}"
            );
            assert!(steps[0] < 0.5, "{peak} nits: {steps:?}");
        }
        // The 100-nit answer is the brighter one in relative terms, because the
        // standard's γ falls below 1 as the panel darkens: mid-code HLG leaves a
        // desktop panel near a third of the scale, a reference display near a fifth.
        let sdr = baked(100.0).sample([0.5; 3], Interpolation::Tetrahedral)[0];
        let hdr = baked(1_000.0).sample([0.5; 3], Interpolation::Tetrahedral)[0];
        assert!(sdr > hdr + 0.1, "{sdr} vs {hdr}");
    }
}
