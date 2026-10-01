//! 1D and 3D colour lookup tables: `.cube` / `.3dl` / `.dat` / `.spi1d` /
//! `.spi3d`, parsing and sampling.

use crate::Result;
use crate::color::log::Log;
use crate::color::primaries::{Primaries, apply, rgb_to_rgb};
use crate::color::tonemap::{ContentLight, DisplayTarget, ToneMap, compress_gamut, tone_map_rgb};
use crate::color::transfer::{Transfer, hlg_ootf_rgb, hlg_system_gamma};
use crate::invalid;

/// How to interpolate between grid nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Interpolation {
    Nearest,
    #[default]
    Trilinear,
    /// Six-tetrahedron split; sharper edges, no ringing on graded ramps.
    Tetrahedral,
}

impl Interpolation {
    /// The word an option takes for this mode.
    pub fn label(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Trilinear => "trilinear",
            Self::Tetrahedral => "tetrahedral",
        }
    }

    /// Read a mode back from its label, taking `linear` as another spelling of
    /// `trilinear` because that is the name the tools outside fvid use.
    pub fn from_label(label: &str) -> Option<Self> {
        Some(match label {
            "nearest" => Self::Nearest,
            "trilinear" | "linear" => Self::Trilinear,
            "tetrahedral" => Self::Tetrahedral,
            _ => return None,
        })
    }

    /// Every mode, in the order a message offers them.
    pub const ALL: [Interpolation; 3] = [Self::Nearest, Self::Trilinear, Self::Tetrahedral];
}

/// A per-channel 1D LUT.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut1d {
    /// `data[channel][code]`, each as authored: a density or log-to-linear table
    /// runs well past the display range, and the overshoot is the signal.
    pub data: [Vec<f32>; 3],
    /// The input range the table was written for, which a `.cube` states per
    /// channel as `DOMAIN_MIN` and `DOMAIN_MAX`.
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
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
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
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
        // The declared range scales the index, never the values: an input at the
        // top of the domain reads the last node whatever numbers that node holds.
        let ch = channel.min(2);
        let lo = self.domain_min[ch];
        let span = (self.domain_max[ch] - lo).max(1e-6);
        let pos = ((value - lo) / span).clamp(0.0, 1.0) * (lut.len() - 1) as f32;
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
    /// Resolve/Adobe `.cube`, including the files that write a 1D table ahead of
    /// the 3D one and the exports that carry a pre-LUT or a shaper in front of
    /// it — those sections are read and dropped, which is what both reference
    /// readers were measured to do. A node outside the display range is kept as
    /// authored, and only the code a graded sample becomes is decided by the ends.
    pub fn from_cube(text: &str) -> Result<Self> {
        let mut one_size = None;
        let mut three_size = None;
        let mut domain_min = [0.0f32; 3];
        let mut domain_max = [1.0f32; 3];
        let mut one_min = None;
        let mut one_max = None;
        let mut three_min = None;
        let mut three_max = None;
        // One file can declare both sizes, so each run of numbers is kept in the
        // bucket its own size line opened; a single list would leave neither
        // count matching.
        let mut one_values: Vec<f32> = Vec::new();
        let mut three_values: Vec<f32> = Vec::new();
        let mut section = None;
        // A pre-LUT or a shaper is a section of its own, ahead of the grid, and
        // FFmpeg's `lut3d` — the reader this repository qualifies against — reads
        // it and drops it: `ffmpeg-preluted-*.rgb` in `tests/fixtures/lut/` are the
        // bytes it wrote for `preluted-17.cube`, a 17-grid with a `LUT_PRELUT_1D_SIZE`
        // pair, a `LUT_PRELUT_3D_SIZE` grid and a `LUT_1D_SHAPER_SIZE` curve written
        // in front of it, and they are `cmp`-identical to the bytes it wrote for the
        // bare grid in all three interpolation modes. That holds with a pre-LUT that
        // maps white to black. So the rows are counted out and thrown away here;
        // leaving them to the grid's bucket would shift the colour instead of leaving
        // it alone, and applying them would move fvid off the reader. OpenColorIO's
        // Resolve reader refuses these files outright, so it decides nothing here.
        let mut skip_rows = 0usize;
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
                one_size = Some(parse_size(rest, line_no, MAX_1D_ENTRIES)?);
                section = Some(false);
                continue;
            }
            if let Some(rest) = upper.strip_prefix("LUT_3D_SIZE") {
                three_size = Some(parse_size(rest, line_no, MAX_3D_SIDE)?);
                section = Some(true);
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
            if let Some(rest) = upper.strip_prefix("LUT_PRELUT_1D_SIZE") {
                skip_rows = parse_size(rest, line_no, MAX_1D_ENTRIES)?;
                continue;
            }
            if let Some(rest) = upper.strip_prefix("LUT_PRELUT_3D_SIZE") {
                // The key states the side, and a section of side `n` runs `n³`
                // rows, so the count to read past is the grid's, not the number on
                // the line.
                skip_rows = parse_size(rest, line_no, MAX_3D_SIDE)?.pow(3);
                continue;
            }
            if let Some(rest) = upper.strip_prefix("LUT_1D_SHAPER_SIZE") {
                skip_rows = parse_size(rest, line_no, MAX_1D_ENTRIES)?;
                continue;
            }
            // The two `*_INPUT_RANGE` keys are the other spelling the input domain
            // has, written as a scalar pair rather than a triple, and each one
            // belongs to its own table. On the 17-grid of `grade-17.cube` with its
            // DOMAIN lines taken out, OCIO's `ociochecklut` answers 0.25 with
            // 0.251519 — and with `LUT_3D_INPUT_RANGE 0.0 0.5` written ahead of it,
            // with 0.531026, which is the same file's own answer at 0.5, so half the
            // declared range is the whole grid; `DOMAIN_MAX 0.5 0.5 0.5` gives the
            // identical number, and `LUT_1D_INPUT_RANGE 0.0 0.5` on that grid leaves
            // it at 0.251519, so the pair belongs to its own table. `ffmpeg lut3d`
            // reads neither key, nor DOMAIN_MIN/MAX, so the domain follows OCIO here
            // the way it already does for DOMAIN_*. A line that writes the pair as
            // two triples is not the pair: OCIO leaves that form unread too.
            let range = upper
                .strip_prefix("LUT_1D_INPUT_RANGE")
                .or_else(|| upper.strip_prefix("LUT_3D_INPUT_RANGE"));
            if let Some(rest) = range {
                let three = upper.starts_with("LUT_3D_INPUT_RANGE");
                if let Some((lo, hi)) = parse_pair(rest) {
                    if three {
                        three_min = Some(lo);
                        three_max = Some(hi);
                    } else {
                        one_min = Some(lo);
                        one_max = Some(hi);
                    }
                }
                continue;
            }
            // The rows of a section this reader drops still have to be read past,
            // or the table beside them swallows them as its own.
            if skip_rows > 0 {
                if is_number_line(line) {
                    skip_rows -= 1;
                }
                continue;
            }
            if is_number_line(line) {
                let rows = match section {
                    Some(false) => &mut one_values,
                    Some(true) => &mut three_values,
                    None => {
                        return Err(invalid(&format!(
                            "a .cube value on line {} precedes any size line",
                            line_no + 1
                        )));
                    }
                };
                rows.extend(
                    line.split_whitespace()
                        .filter_map(|t| t.parse::<f32>().ok()),
                );
                continue;
            }
            // What is left is a key this reader has no clause for, and the file
            // keeps opening: `LUT_TYPE 3D`, `VERTEX_FORMAT UNSIGNED_BYTE`,
            // `NUM_SAMPLES 100`, `LUT_3D_OUTPUT_RANGE` and a line written
            // `BOGUS_KEY 3` all leave `ffmpeg lut3d` answering with the grid alone,
            // byte for byte the same frame it writes without them. OpenColorIO's
            // Resolve reader rejects each of them instead — it is FFmpeg that decides
            // this. Refusing a key would lock out a file whose colour is settled, and
            // accepting one cannot quiet a real mistake: the rows a section needs are
            // counted against its own size line, so numbers belonging to an unknown
            // key land in the bucket of the table beside them and fail that table's
            // count — an error, not a wrong colour.
        }
        let nan = one_values
            .iter()
            .chain(&three_values)
            .any(|v| !v.is_finite());
        if nan {
            return Err(invalid("a .cube value is not a number"));
        }
        if let Some(n) = three_size {
            // Both tables in one file is how a grading suite exports a cube that
            // expects a curve in front of it, and no document settles which way
            // round the two go. The grid is what carries the colour decision, so
            // it is what gets applied — as FFmpeg's `lut3d` was measured to do
            // with the same file.
            if three_values.len() != n * n * n * 3 {
                return Err(invalid(&format!(
                    "3D LUT expected {} values, got {}",
                    n * n * n * 3,
                    three_values.len()
                )));
            }
            // A node may sit outside the display range — the `.cube` exports in
            // circulation run from −0.02 to 1.07, and a log-to-linear table
            // beyond that — and the overshoot is the look: it is what makes the
            // interpolators bend the shoulder instead of flattening it. So the
            // value is stored as authored, and only the code a graded sample
            // becomes is decided by the ends. `ffmpeg`'s `lut3d` was measured to
            // do exactly this: a size-2 cube with a red node at 1.5 puts red 192
            // at mid-grey, half of 1.5, where clipping the node on the way in
            // could only ever return 128.
            let data = three_values
                .chunks_exact(3)
                .map(|c| [c[0], c[1], c[2]])
                .collect();
            return Ok(Self::Three(Lut3d {
                size: n,
                domain_min: three_min.unwrap_or(domain_min),
                domain_max: three_max.unwrap_or(domain_max),
                data,
            }));
        }
        if let Some(n) = one_size {
            let rows = if one_values.len() == n * 3 {
                3
            } else if one_values.len() == n {
                1
            } else {
                return Err(invalid(&format!(
                    "1D LUT expected {n} or {} values, got {}",
                    n * 3,
                    one_values.len()
                )));
            };
            let mut data = [
                Vec::with_capacity(n),
                Vec::with_capacity(n),
                Vec::with_capacity(n),
            ];
            // The values are the table's output and stay as they are, whether
            // that is 100 or −0.05; the domain belongs to its input, so it goes
            // to the sampler rather than being divided into the rows here.
            for chunk in one_values.chunks(rows) {
                for ch in 0..3 {
                    data[ch].push(if rows == 1 { chunk[0] } else { chunk[ch] });
                }
            }
            return Ok(Self::One(Lut1d {
                data,
                domain_min: one_min.unwrap_or(domain_min),
                domain_max: one_max.unwrap_or(domain_max),
            }));
        }
        Err(invalid("a .cube must declare LUT_1D_SIZE or LUT_3D_SIZE"))
    }

    /// Autodesk/Avid `.3dl`: 12-bit integer rows listing the grid with the blue
    /// axis fastest — the reverse of a `.cube`. The grid side is declared either
    /// by a line holding a single size, or, as Autodesk's and Color Finesse's
    /// own writers emit it, by a mesh line of `size` input code values behind
    /// header keywords such as `3DMESH` and `Mesh 4 12`. A file that states
    /// neither is read from its row count, which has to be an exact cube. A
    /// mesh line is only read as one while no row has been seen and it is not
    /// three values wide, since a row and the mesh of a three-node grid look
    /// alike. Codes are 12-bit unless a `Mesh <in> <out>` line declares the
    /// output depth the file was written at, which is the only way a 16-bit
    /// `.3dl` says so.
    pub fn from_3dl(text: &str) -> Result<Self> {
        let mut rows: Vec<[f32; 3]> = Vec::new();
        let mut size = None;
        // 12-bit codes unless the header says otherwise, which is what the
        // writers that state nothing mean: 35 of the 36 files measured hold
        // values up to 4095, and only the one `Mesh 4 16` file goes beyond it.
        let mut scale = 4095.0f32;
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields[0].parse::<f32>().is_err() {
                // A header line: `3DMESH`, `LUT8`, `Mesh 4 12`, `FROM 0 4095`.
                // None of it is a node value, and the grid says for itself what
                // the mesh means, so it is read and dropped — except for `Mesh`,
                // whose second number is the one thing here a reader needs and
                // the rows cannot tell it: the depth the codes are written at.
                if fields.len() == 3 && fields[0].eq_ignore_ascii_case("Mesh") {
                    if let Ok(bits) = fields[2].parse::<u32>() {
                        if (4..=24).contains(&bits) {
                            scale = (2.0f32).powi(bits as i32) - 1.0;
                        }
                    }
                }
                continue;
            }
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
            let numeric = fields.iter().all(|f| f.parse::<f32>().is_ok());
            if !numeric {
                return Err(invalid(&format!("bad .3dl value on line {}", line_no + 1)));
            }
            if fields.len() != 3 && size.is_none() && rows.is_empty() {
                // The mesh line: one input code value per node along an axis,
                // which is the declaration the grid size is read from. Rows are
                // three fields wide, so a line of any other width that comes
                // before any row can only be this.
                let n = fields.len();
                if !(2..=64).contains(&n) {
                    return Err(invalid("a .3dl size must be 2..=64"));
                }
                size = Some(n);
                continue;
            }
            if fields.len() != 3 {
                return Err(invalid(&format!(
                    "a .3dl row needs 3 channels on line {}",
                    line_no + 1
                )));
            }
            let mut v = [0.0f32; 3];
            for (i, f) in fields.iter().enumerate() {
                let n: f32 = f
                    .parse()
                    .map_err(|_| invalid(&format!("bad .3dl value `{f}`")))?;
                v[i] = (n / scale).clamp(0.0, 1.0);
            }
            rows.push(v);
        }
        let size = match size {
            Some(n) => n,
            // Some writers state neither: no size line and no mesh line, just
            // the rows. Their count is then the only declaration the file has,
            // and only an exact cube can be a grid.
            None => (2..=64).find(|n| n * n * n == rows.len()).ok_or_else(|| {
                invalid(&format!(
                    "a .3dl declares no size and its {} rows are not a grid",
                    rows.len()
                ))
            })?,
        };
        if rows.len() != size * size * size {
            return Err(invalid(&format!(
                "a .3dl expected {} rows, got {}",
                size * size * size,
                rows.len()
            )));
        }
        // The file lists the grid with blue fastest; the table does not.
        let data = blue_fastest_to_slots(rows, size);
        Ok(Self::Three(Lut3d {
            size,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            data,
        }))
    }

    /// Iridas `.dat`: one optional `3DLUTSIZE <side>` line and then side³ rows
    /// of three floats each, with nothing else in the file — no index in the
    /// row, no brace, no keyword but that one line, and blank or `#`-comment
    /// lines anywhere between them. `ffmpeg`'s `lut3d` is the only
    /// implementation that reads it, so its `parse_dat` is the whole grammar,
    /// and the one thing that grammar does not say is which axis runs fastest.
    /// `ffmpeg`'s own array answers it: `parse_dat` puts its innermost loop in
    /// the ones place of the slot `r·size² + g·size + b`, so the rows run blue
    /// fastest — the same order a `.3dl` uses and the reverse of a `.cube`, so
    /// this reader transposes like that one does.
    ///
    /// Measured, not inferred: the rows of the 17-grid look were written out in
    /// all six assignments of the file's three digit places to red, green and
    /// blue, and `ffmpeg` was run on each. Only the blue-fastest file reproduces
    /// its own `.cube` output — all 12 288 probe bytes, in all three
    /// interpolations. The five others differ on 10 542 to 12 184 of them
    /// (`tests/fixtures/lut/README.md`), which is the width of the mistake a
    /// reader cannot talk itself out of.
    ///
    /// Three places are held tighter than `parse_dat`, and the first two are the
    /// places where it reads the wrong file rather than a different one: it takes
    /// the first three numbers of a wider row and stops at the last row it needs,
    /// so a 17-grid with a fourth column or a stray row behind it passes there
    /// and is refused here. The third is the grid bound every format here shares
    /// — 128 nodes to a side, where `ffmpeg` goes to 256 and a file that large is
    /// 16 777 216 lines of text.
    ///
    /// Values are taken as written and unscaled, so a table that overshoots white
    /// keeps its headroom, and a file with no directive is read at whatever exact
    /// cube its rows make rather than at the 33 nodes `ffmpeg` fixes them to.
    pub fn from_dat(text: &str) -> Result<Self> {
        let mut rows: Vec<[f32; 3]> = Vec::new();
        let mut declared = None;
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields[0].eq_ignore_ascii_case("3DLUTSIZE") {
                if declared.is_some() || !rows.is_empty() {
                    return Err(invalid(
                        "a .dat states 3DLUTSIZE twice, or after its rows have begun",
                    ));
                }
                let Some(value) = fields.get(1) else {
                    return Err(invalid(&format!(
                        "3DLUTSIZE on line {} carries no number",
                        line_no + 1
                    )));
                };
                let Ok(n) = value.parse::<usize>() else {
                    return Err(invalid(&format!(
                        "bad .dat size `{value}` on line {}",
                        line_no + 1
                    )));
                };
                if !(2..=MAX_3D_SIDE).contains(&n) {
                    return Err(invalid(&format!(
                        "a .dat size must be 2..={MAX_3D_SIDE}, line {} says {n}",
                        line_no + 1
                    )));
                }
                declared = Some(n);
                continue;
            }
            if fields.len() != 3 {
                return Err(invalid(&format!(
                    "a .dat row needs 3 channels on line {}, it has {}",
                    line_no + 1,
                    fields.len()
                )));
            }
            let mut v = [0.0f32; 3];
            for (i, f) in fields.iter().enumerate() {
                let Ok(n) = f.parse::<f32>() else {
                    return Err(invalid(&format!(
                        "bad .dat value `{f}` on line {}",
                        line_no + 1
                    )));
                };
                v[i] = n;
            }
            if !v.iter().all(|c| c.is_finite()) {
                return Err(invalid(&format!(
                    "a .dat value on line {} is not a number",
                    line_no + 1
                )));
            }
            rows.push(v);
        }
        let size = match declared {
            Some(n) => n,
            // No directive: the row count is the only declaration the file has.
            // `ffmpeg` fixes such a file at 33 nodes; an exact cube says for
            // itself what it is, which is the rule the `.3dl` reader uses.
            None => (2..=MAX_3D_SIDE)
                .find(|n| n * n * n == rows.len())
                .ok_or_else(|| {
                    invalid(&format!(
                        "a .dat states no 3DLUTSIZE and its {} rows are not a grid",
                        rows.len()
                    ))
                })?,
        };
        if rows.len() != size * size * size {
            return Err(invalid(&format!(
                "a .dat expected {} rows, got {}",
                size * size * size,
                rows.len()
            )));
        }
        Ok(Self::Three(Lut3d {
            size,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            data: blue_fastest_to_slots(rows, size),
        }))
    }

    /// SPI `.spi1d`: the header tags `Version`, `From`, `Length` and
    /// `Components` in any order, a `{`, then `Length` rows of `Components`
    /// values each. A row's index is its position in the file rather than a
    /// number in the row, which is why a 4 101-entry table is 4 101 lines long.
    /// `From` is one pair shared by every channel, so it lands on all three
    /// domains; the format states no per-channel input range a reader could
    /// disagree about. The value rules are OpenColorIO's, which is the only
    /// specification this format has: one value is the same curve on all three
    /// channels, two values leave blue at nothing, three are red, green, blue.
    /// `Components 0` is refused rather than read as the empty table the
    /// reference reader makes of it.
    pub fn from_spi1d(text: &str) -> Result<Self> {
        let mut version = false;
        let mut length = None;
        let mut components = None;
        let mut from = [0.0f32, 1.0];
        let mut rows: Vec<[f32; 3]> = Vec::new();
        let mut body = false;
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if !body {
                if line.starts_with('{') {
                    body = true;
                    continue;
                }
                // A header line that names none of the four tags holds a tag
                // this format has no meaning for, which is what the reference
                // reader does with it too: read on and keep the table.
                if let Some(rest) = spi_tag(line, "Version") {
                    let stated = rest
                        .split_whitespace()
                        .next()
                        .is_some_and(|t| t.parse::<i64>().is_ok());
                    if !stated {
                        return Err(invalid(&format!("bad Version on line {}", line_no + 1)));
                    }
                    version = true;
                } else if let Some(rest) = spi_tag(line, "From") {
                    let nums: Vec<f32> = rest
                        .split_whitespace()
                        .take(2)
                        .map(|t| {
                            t.parse::<f32>().map_err(|_| {
                                invalid(&format!("bad From value on line {}", line_no + 1))
                            })
                        })
                        .collect::<Result<_>>()?;
                    if nums.len() != 2 {
                        return Err(invalid(&format!(
                            "From needs two numbers on line {}",
                            line_no + 1
                        )));
                    }
                    from = [nums[0], nums[1]];
                } else if let Some(rest) = spi_tag(line, "Length") {
                    let n: usize = rest
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .parse()
                        .map_err(|_| invalid(&format!("bad Length on line {}", line_no + 1)))?;
                    if !(2..=MAX_1D_ENTRIES).contains(&n) {
                        return Err(invalid(&format!(
                            "a .spi1d Length must be 2..={MAX_1D_ENTRIES}"
                        )));
                    }
                    length = Some(n);
                } else if let Some(rest) = spi_tag(line, "Components") {
                    let n: usize = rest
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .parse()
                        .map_err(|_| invalid(&format!("bad Components on line {}", line_no + 1)))?;
                    if !(1..=3).contains(&n) {
                        return Err(invalid("a .spi1d Components must be 1, 2 or 3"));
                    }
                    components = Some(n);
                }
                continue;
            }
            if line.starts_with('}') {
                break;
            }
            let Some(n) = components else {
                return Err(invalid(&format!(
                    "a .spi1d row on line {} precedes its Components tag",
                    line_no + 1
                )));
            };
            let fields: Vec<f32> = line
                .split_whitespace()
                .map(|t| {
                    t.parse::<f32>()
                        .map_err(|_| invalid(&format!("bad .spi1d value on line {}", line_no + 1)))
                })
                .collect::<Result<_>>()?;
            if fields.len() != n {
                return Err(invalid(&format!(
                    "line {} holds {} values and the table declares {n}",
                    line_no + 1,
                    fields.len()
                )));
            }
            let mut v = [fields[0]; 3];
            if n >= 2 {
                v[1] = fields[1];
                v[2] = if n == 2 { 0.0 } else { fields[2] };
            }
            rows.push(v);
        }
        if !version {
            return Err(invalid("a .spi1d carries no Version tag"));
        }
        let len = length.ok_or_else(|| invalid("a .spi1d carries no Length tag"))?;
        if components.is_none() {
            return Err(invalid("a .spi1d carries no Components tag"));
        }
        if rows.len() != len {
            return Err(invalid(&format!(
                "a .spi1d declares {len} entries and holds {}",
                rows.len()
            )));
        }
        if rows.iter().flatten().any(|v| !v.is_finite()) {
            return Err(invalid("a .spi1d value is not a number"));
        }
        let mut data = [
            Vec::with_capacity(len),
            Vec::with_capacity(len),
            Vec::with_capacity(len),
        ];
        for row in rows {
            for ch in 0..3 {
                data[ch].push(row[ch]);
            }
        }
        Ok(Self::One(Lut1d {
            data,
            domain_min: [from[0]; 3],
            domain_max: [from[1]; 3],
        }))
    }

    /// SPI `.spi3d`: a `SPILUT` head, the tag line every writer puts behind it,
    /// three equal axis sizes, then rows of `r g b R G B` — a node's own indices
    /// ahead of its values, so no order is asked of the file, a node written
    /// twice is an error instead of the last row winning, and a node never
    /// written is caught by the row count. Values keep their overshoot exactly
    /// as a `.cube` keeps it. The tag line is consumed whether or not it says
    /// anything, which is what the reference reader does with it, so a writer
    /// that leaves it out is refused by both.
    pub fn from_spi3d(text: &str) -> Result<Self> {
        let mut lines = text.lines().enumerate();
        let (_, head) =
            spi_line(&mut lines).ok_or_else(|| invalid("an empty file is not a .spi3d"))?;
        if !head
            .split_whitespace()
            .next()
            .is_some_and(|t| t.eq_ignore_ascii_case("SPILUT"))
        {
            return Err(invalid(&format!(
                "a .spi3d must open with SPILUT, found {head:?}"
            )));
        }
        let _ = spi_line(&mut lines);
        let (size_no, size_line) =
            spi_line(&mut lines).ok_or_else(|| invalid("a .spi3d declares no grid size"))?;
        let fields: Vec<&str> = size_line.split_whitespace().collect();
        if fields.len() != 3 {
            return Err(invalid(&format!(
                "a .spi3d size line {} must hold three numbers",
                size_no + 1
            )));
        }
        let mut side = [0usize; 3];
        for (i, f) in fields.iter().enumerate() {
            side[i] = f
                .parse()
                .map_err(|_| invalid(&format!("bad .spi3d size on line {}", size_no + 1)))?;
        }
        // Three axes are named and one grid is held: the format can state a box
        // that is not a cube, and neither this table nor the reference reader
        // can fill one — it is the thing that format refuses outright.
        if side[1] != side[0] || side[2] != side[0] {
            return Err(invalid(&format!(
                "a .spi3d must be a cube, its axes on line {} are {side:?}",
                size_no + 1
            )));
        }
        if !(2..=MAX_3D_SIDE).contains(&side[0]) {
            return Err(invalid(&format!(
                "a .spi3d size must be 2..={MAX_3D_SIDE}, line {} says {}",
                size_no + 1,
                side[0]
            )));
        }
        let size = side[0];
        let mut data = vec![[0.0f32; 3]; size * size * size];
        let mut filled = vec![false; data.len()];
        let mut rows = 0usize;
        for (line_no, raw) in lines {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() != 6 {
                return Err(invalid(&format!(
                    "a .spi3d row is three indices and three values, line {} has {}",
                    line_no + 1,
                    fields.len()
                )));
            }
            let mut idx = [0usize; 3];
            let mut val = [0.0f32; 3];
            for (i, f) in fields.iter().enumerate() {
                if i < 3 {
                    let n: i64 = f.parse().map_err(|_| {
                        invalid(&format!("bad .spi3d index `{f}` on line {}", line_no + 1))
                    })?;
                    if n < 0 || n >= size as i64 {
                        return Err(invalid(&format!(
                            "a .spi3d index {n} on line {} is outside the {size}³ grid",
                            line_no + 1
                        )));
                    }
                    idx[i] = n as usize;
                } else {
                    val[i - 3] = f.parse().map_err(|_| {
                        invalid(&format!("bad .spi3d value `{f}` on line {}", line_no + 1))
                    })?;
                }
            }
            if !val.iter().all(|v| v.is_finite()) {
                return Err(invalid(&format!(
                    "a .spi3d value on line {} is not a number",
                    line_no + 1
                )));
            }
            let slot = idx[0] + size * (idx[1] + size * idx[2]);
            if filled[slot] {
                return Err(invalid(&format!(
                    "a .spi3d node {} {} {} on line {} is written twice",
                    idx[0],
                    idx[1],
                    idx[2],
                    line_no + 1
                )));
            }
            filled[slot] = true;
            data[slot] = val;
            rows += 1;
        }
        if rows != data.len() {
            return Err(invalid(&format!(
                "a .spi3d expected {} rows, got {rows}",
                data.len()
            )));
        }
        Ok(Self::Three(Lut3d {
            size,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            data,
        }))
    }

    /// Read any of the five formats from content, which is how `--lut` takes a
    /// path without asking its extension for a hint: a `.cube` always names a
    /// size keyword, a `.spi3d` always opens with `SPILUT`, a `.dat` states
    /// `3DLUTSIZE` or is nothing but normalised rows, a `.spi1d` is the only one
    /// with a brace, and what has none of those is a `.3dl`.
    pub fn from_text(text: &str) -> Result<Self> {
        // Each test here is one the format it selects must always pass, so a
        // file cannot be handed to the wrong reader by it. The head is the first
        // line that carries anything, comments included, since a `.dat` puts its
        // `3DLUTSIZE` behind them the way `ffmpeg` reads it.
        let head = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .unwrap_or("");
        if head
            .split_whitespace()
            .next()
            .is_some_and(|t| t.eq_ignore_ascii_case("SPILUT"))
        {
            return Self::from_spi3d(text);
        }
        if text.contains("_SIZE") {
            return Self::from_cube(text);
        }
        if head
            .split_whitespace()
            .next()
            .is_some_and(|t| t.eq_ignore_ascii_case("3DLUTSIZE"))
            || is_normalised_dat(text)
        {
            return Self::from_dat(text);
        }
        if text.contains('{') {
            return Self::from_spi1d(text);
        }
        // What arrives with none of those marks is the `.cube` extension taken by
        // some format entirely — Gaussian volumetric dumps, git-lfs pointers,
        // prose — so name every format ruled out and quote the line that gives
        // the file away.
        Self::from_3dl(text).map_err(|error| {
            invalid(&format!(
                "not a LUT: no .cube size line, no .spi3d SPILUT head, no .dat 3DLUTSIZE and no .spi1d brace ({error}); it starts {head:?}"
            ))
        })
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
    ///
    /// A 3D grid is compared node by node, not along its grey diagonal: a
    /// conversion that only changes the triangle maps every grey to itself, so
    /// the diagonal cannot see the change the whole picture is made of.
    pub fn is_identity(&self, tol: f32) -> bool {
        match self {
            // Three per-channel curves hide nothing from a grey: one value fed
            // to all three has to come back the same value three times.
            Self::One(_) => (0..=16).all(|i| {
                let v = i as f32 / 16.0;
                let out = self.sample([v, v, v], Interpolation::Tetrahedral);
                out.iter().zip([v, v, v]).all(|(a, b)| (a - b).abs() <= tol)
            }),
            Self::Three(l) => {
                let s = l.size;
                let step = 1.0 / (s - 1) as f32;
                (0..s).all(|b| {
                    (0..s).all(|g| {
                        (0..s).all(|r| {
                            let node = l.data[r + s * (g + s * b)];
                            let want = [
                                l.domain_min[0]
                                    + r as f32 * step * (l.domain_max[0] - l.domain_min[0]),
                                l.domain_min[1]
                                    + g as f32 * step * (l.domain_max[1] - l.domain_min[1]),
                                l.domain_min[2]
                                    + b as f32 * step * (l.domain_max[2] - l.domain_min[2]),
                            ];
                            node.iter().zip(want).all(|(a, w)| (a - w).abs() <= tol)
                        })
                    })
                })
            }
        }
    }

    /// Serialise back to `.cube`, so a generated LUT survives a write/read cycle.
    pub fn to_cube(&self) -> String {
        let mut out = String::new();
        match self {
            Self::One(l) => {
                out.push_str(&format!("LUT_1D_SIZE {}\n", l.len()));
                if l.domain_min != [0.0; 3] || l.domain_max != [1.0; 3] {
                    out.push_str(&format!(
                        "DOMAIN_MIN {} {} {}\nDOMAIN_MAX {} {} {}\n",
                        l.domain_min[0],
                        l.domain_min[1],
                        l.domain_min[2],
                        l.domain_max[0],
                        l.domain_max[1],
                        l.domain_max[2],
                    ));
                }
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

/// The declared side of a 3D grid: 128 is 2,1 million nodes, and a grid's own
/// memory is why a grid is bounded at all.
const MAX_3D_SIDE: usize = 128;

/// The declared length of a 1D table, which is a list rather than a cube: log
/// and density curves are written with an entry per code of the *encoded*
/// signal, so 4096 is ordinary and the value lines settle the count anyway.
/// OpenColorIO's ceiling for a 1D table is 300 000 against 129 for a grid.
const MAX_1D_ENTRIES: usize = 300_000;

fn parse_size(rest: &str, line_no: usize, limit: usize) -> Result<usize> {
    let n: usize = rest
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .parse()
        .map_err(|_| invalid(&format!("bad LUT size on line {}", line_no + 1)))?;
    if !(2..=limit).contains(&n) {
        return Err(invalid(&format!("LUT size must be 2..={limit}")));
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

/// A key's scalar pair, which is how `LUT_1D_INPUT_RANGE` and
/// `LUT_3D_INPUT_RANGE` state the input domain: two numbers, the ends, applied to
/// every channel. Any other count on that line is not the pair and is left alone,
/// as OCIO leaves the six-number form of it alone.
fn parse_pair(rest: &str) -> Option<([f32; 3], [f32; 3])> {
    let nums: Vec<f32> = rest
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    if nums.len() != 2 {
        return None;
    }
    Some(([nums[0]; 3], [nums[1]; 3]))
}

fn is_number_line(line: &str) -> bool {
    line.split_whitespace()
        .next()
        .is_some_and(|t| t.parse::<f32>().is_ok())
}

/// Re-index a grid whose file lists the blue axis fastest into the table's own
/// layout, where red runs fastest: line `b + size·(g + size·r)` holds node
/// (r, g, b), which the sampler reads at `r + size·(g + size·b)`. Both the
/// `.3dl` and the `.dat` reader need it, and neither of them guesses the order:
/// each has it measured against `ffmpeg`.
fn blue_fastest_to_slots(rows: Vec<[f32; 3]>, size: usize) -> Vec<[f32; 3]> {
    let mut data = vec![[0.0f32; 3]; rows.len()];
    for (i, row) in rows.into_iter().enumerate() {
        let b = i % size;
        let g = i / size % size;
        let r = i / (size * size);
        data[r + size * (g + size * b)] = row;
    }
    data
}

/// Above this, three numbers to a line are node codes rather than a normalised
/// table: a `.3dl` writes its nodes at 2^bits−1 — 4 095 for the files here, and
/// never below 15 for a grid that says its mesh — while a `.dat` writes them at
/// one and the headroom above it.
const DAT_NORMALISED_CEILING: f32 = 1.5;

/// A `.dat` that states no `3DLUTSIZE` is a file of nothing but rows: an exact
/// cube of lines, three values on each, every one of them normalised. A `.3dl`
/// is that same shape at another scale — its nodes are codes up to 2^bits−1,
/// a `.dat`'s are fractions of white — and since both list the grid with blue
/// fastest, nothing but the scale can tell them apart. It does, wherever the
/// `.3dl` uses a code of 2 or more; a table written at 4-bit output that never
/// leaves 0 and 1 is the one file the sniff cannot separate, and it is not a
/// grade.
fn is_normalised_dat(text: &str) -> bool {
    let mut rows = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() != 3 {
            return false;
        }
        for f in fields {
            let Ok(n) = f.parse::<f32>() else {
                return false;
            };
            if !n.is_finite() || n.abs() > DAT_NORMALISED_CEILING {
                return false;
            }
        }
        rows += 1;
    }
    (2..=MAX_3D_SIDE).any(|n| n * n * n == rows)
}

/// The next line of a SPI file that holds anything, with its leading and
/// trailing space gone, leaving the reader on the line after it.
fn spi_line<'a>(lines: &mut impl Iterator<Item = (usize, &'a str)>) -> Option<(usize, &'a str)> {
    lines.find(|(_, raw)| !raw.trim().is_empty())
}

/// The remainder of a header line behind one of the SPI tags, whose name the
/// writers may spell without the space before its value.
fn spi_tag<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    line.get(..tag.len())
        .is_some_and(|t| t.eq_ignore_ascii_case(tag))
        .then(|| line[tag.len()..].trim())
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
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
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
        // What a curve's 1.0 stands for, in cd/m². A log curve states it at
        // diffuse white, an SDR video curve has no absolute scale at all and
        // takes the panel's white, PQ is absolute and already is one. HLG is the
        // one HDR curve whose 1.0 means "whatever this panel reaches": BT.2100
        // makes its scene light display-size dependent and lets γ follow the
        // panel, so fixing HLG to the 1 000 cd/m² reference here would claim ten
        // times the headroom a 100-nit destination has and clip every code above
        // 0.45 to white.
        let full_scale = |curve: Transfer| match curve {
            Transfer::Hlg => panel,
            curve => curve.full_scale_nits(self.sdr_white_nits),
        };
        let scale = match self.log {
            Some(_) => self.sdr_white_nits,
            None => full_scale(self.from),
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
                // No compression means no panel to fit the light into, but the
                // two curves still disagree about how much light a code carries:
                // read the source's linear as cd/m² and hand the destination
                // curve its own fraction of that. Where the families agree the
                // ratio is one and the conversion stays what it was.
                None => {
                    let to = full_scale(self.to);
                    mapped.map(|v| v * (scale / to))
                }
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

    /// A pre-LUT is a section the writers put in front of the grid, and FFmpeg's
    /// `lut3d` was measured to read it and drop it: the bytes it writes for a grid
    /// with a `LUT_PRELUT_1D_SIZE` pair, a `LUT_PRELUT_3D_SIZE` grid and a
    /// `LUT_1D_SHAPER_SIZE` curve ahead of it are `cmp`-identical to the bytes it
    /// writes for the bare grid in all three modes, including a pair that maps white
    /// to black (`tests/fixtures/lut/preluted-17.cube`). The rows are read past here,
    /// so the grid keeps both its colour and its count.
    #[test]
    fn a_pre_lut_section_is_read_past_and_leaves_the_grid_alone() {
        let grid = "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n1 0 0\n1 0 1\n0 1 0\n0 1 1\n1 1 0\n1 1 1\n";
        let plain = Lut::from_cube(grid).unwrap();
        for head in [
            "LUT_PRELUT_1D_SIZE 2\n0 0 0\n0 0 0\n",
            "LUT_PRELUT_1D_SIZE 2\n1 1 1\n0 0 0\n",
            "LUT_PRELUT_3D_SIZE 2\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n1 1 1\n1 1 1\n1 1 1\n1 1 1\n",
            "LUT_1D_SHAPER_SIZE 4\n0 0 0\n0.3 0.3 0.3\n0.7 0.7 0.7\n1 1 1\n",
        ] {
            let with = Lut::from_cube(&format!("{head}{grid}"))
                .unwrap_or_else(|e| panic!("{head:?}: {e}"));
            assert_eq!(with, plain, "{head:?}");
            for input in [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ] {
                assert_eq!(
                    with.sample(input, Interpolation::Nearest),
                    plain.sample(input, Interpolation::Nearest),
                    "{head:?} at {input:?}"
                );
            }
        }
    }

    /// A key this reader has no clause for does not close the file, because the
    /// reader this repository qualifies against lets it move nothing: with
    /// `LUT_TYPE 3D`, `VERTEX_FORMAT UNSIGNED_BYTE`, `NUM_SAMPLES 100` and a
    /// `LUT_3D_OUTPUT_RANGE` triple ahead of the same grid, `ffmpeg lut3d` writes
    /// the frame it writes without them, byte for byte. OpenColorIO's Resolve reader
    /// rejects each of those keys instead, so FFmpeg decides this: the file is read
    /// as though the key were a comment.
    #[test]
    fn a_vendor_key_line_does_not_move_the_grid_or_close_the_file() {
        let grid = "LUT_3D_SIZE 2\n0.1 0.1 0.1\n0.1 0.1 0.1\n0.1 0.1 0.1\n0.1 0.1 0.1\n\
                    0.1 0.1 0.1\n0.1 0.1 0.1\n0.1 0.1 0.1\n0.9 0.9 0.9\n";
        for head in [
            "LUT_TYPE 3D",
            "VERTEX_FORMAT UNSIGNED_BYTE",
            "NUM_SAMPLES 100",
            "LUT_3D_OUTPUT_RANGE 0.0 0.0 0.0",
            "BOGUS_KEY 3",
        ] {
            let lut = Lut::from_cube(&format!("{head}\n{grid}")).unwrap();
            assert_eq!(
                lut.sample([1.0, 1.0, 1.0], Interpolation::Nearest),
                [0.9, 0.9, 0.9],
                "{head}"
            );
        }
    }

    /// `LUT_3D_INPUT_RANGE a b` is the grid's domain in the other spelling, and
    /// OCIO applies it: on a `LUT_3D_SIZE 2` identity grid whose range is written
    /// 0.0 0.5, `ociochecklut` answers 0.25 with 0.5, half of the declared range
    /// being the whole grid. The same pair under `LUT_1D_INPUT_RANGE` on that grid
    /// leaves the answer at 0.25, and the six-number form of the grid's own key
    /// leaves it there too, so only a table's own scalar pair is read. `ffmpeg
    /// lut3d` reads neither of them, nor DOMAIN_MIN/MAX, so the domain follows
    /// OCIO the way it already does for DOMAIN_*.
    #[test]
    fn an_input_range_pair_is_read_as_the_domain_of_its_own_table() {
        let grid = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        for (head, expected) in [
            ("LUT_3D_INPUT_RANGE 0.0 0.5", [0.5; 3]),
            ("LUT_1D_INPUT_RANGE 0.0 0.5", [0.25; 3]),
            ("LUT_3D_INPUT_RANGE 0 0 0 0.5 0.5 0.5", [0.25; 3]),
        ] {
            let lut = Lut::from_cube(&format!("{head}\n{grid}")).unwrap();
            assert_eq!(
                lut.sample([0.25; 3], Interpolation::Trilinear),
                expected,
                "{head}"
            );
        }
        // The 1D key on a 1D table is its domain as well, pinned on the two
        // answers OCIO gives for the same file: 0.25 lands on the middle entry and
        // 0.5 on the top one, the range being half of what the codes say.
        let one =
            "LUT_1D_SIZE 3\nLUT_1D_INPUT_RANGE 0.0 0.5\n0.0 0.0 0.0\n0.5 0.5 0.5\n1.0 1.0 0.0\n";
        let lut = Lut::from_cube(one).unwrap();
        assert_eq!(lut.sample([0.25; 3], Interpolation::Trilinear), [0.5; 3]);
        assert_eq!(
            lut.sample([0.5; 3], Interpolation::Trilinear),
            [1.0, 1.0, 0.0]
        );
    }

    /// A file that writes both tables is a grid with a curve in front of it, and
    /// no document settles which way round the two go. The grid carries the
    /// colour decision, so it is what gets applied — the same result FFmpeg's
    /// `lut3d` gives for this file, measured on black, red, green and blue.
    #[test]
    fn a_cube_with_both_tables_is_read_for_its_3d_grid() {
        let text = "LUT_1D_SIZE 2\n1 1 1\n0 0 0\nLUT_3D_SIZE 2\n\
                    0 0 0\n0 0 1\n1 0 0\n1 0 1\n0 1 0\n0 1 1\n1 1 0\n1 1 1\n";
        let lut = Lut::from_cube(text).unwrap();
        assert!(matches!(lut, Lut::Three(_)));
        assert_eq!(lut.size(), 2);
        for (input, expected) in [
            ([0.0; 3], [0.0; 3]),
            ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
            ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ] {
            assert_eq!(
                lut.sample(input, Interpolation::Nearest),
                expected,
                "{input:?}"
            );
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
    fn a_node_outside_the_display_range_survives_the_parse() {
        // Red authored to run from -0.2 to 1.5, green and blue plain ramps: the
        // middle of the ramp then reads 0.65, which is (-0.2 + 1.5) / 2, not the
        // 0.5 a table clipped on the way in can only give. This is the shape the
        // `ffmpeg` oracle in tests/lut_ffmpeg.rs measures.
        let mut text = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text.push_str(&format!("{} {g} {b}\n", if r == 0 { -0.2 } else { 1.5 }));
                }
            }
        }
        let Lut::Three(l) = Lut::from_cube(&text).unwrap() else {
            panic!("3d");
        };
        assert_eq!(l.data[0][0], -0.2);
        assert_eq!(l.data[7][0], 1.5);
        let mid = l.sample([0.5, 0.5, 0.5], Interpolation::Trilinear);
        assert!((mid[0] - 0.65).abs() < 1e-6, "{mid:?}");
        assert!((mid[1] - 0.5).abs() < 1e-6, "{mid:?}");
        // The writer keeps it too, so the file round trips rather than drifts.
        assert!(Lut::Three(l).to_cube().contains("1.5"));
    }

    /// A 1D table's `DOMAIN_MIN`/`DOMAIN_MAX` state the range of its *input*,
    /// the same as a 3D grid's: the file abpy ships for negative printing says
    /// so in its own header ("# input: log10/density, output: linear") and its
    /// 1024 rows then fit 10^(in-2) over 0..4 to 5e-9, while its values run from
    /// 0.01 to 100 — a range no display code lives in. Autodesk's reader in
    /// OpenColorIO agrees for both kinds: the declared range becomes an op in
    /// front of the table, and the table's own numbers are never rescaled.
    #[test]
    fn a_1d_domain_is_the_input_range_and_its_values_are_left_alone() {
        let mut text = String::from("LUT_1D_SIZE 5\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 4 4 4\n");
        for i in 0..5 {
            let v = 10f32.powf(4.0 * i as f32 / 4.0 - 2.0);
            text.push_str(&format!("{v} {v} {v}\n"));
        }
        let Lut::One(l) = Lut::from_cube(&text).unwrap() else {
            panic!("1d");
        };
        assert_eq!(l.domain_max, [4.0; 3]);
        assert_eq!(l.data[0][0], 0.01);
        assert_eq!(l.data[0][4], 100.0, "a 1D table is not a display range");
        // An input of 1.0 is a quarter of the declared domain, i.e. the second
        // node of this five-node table.
        assert_eq!(l.sample(0, 1.0), 0.1);
        assert_eq!(l.sample(0, 0.0), 0.01);
        // Outside the domain the sampler holds at the ends, as it does for 3D.
        assert_eq!(l.sample(0, 4.0), 100.0);
        assert_eq!(l.sample(0, 2.0), 1.0);
    }

    /// A log-to-linear table is the same case with the domain left at its
    /// default: superwhite is the whole point, so a value above 1.0 is the
    /// signal, not a file to be repaired. Apple's own export runs from −0.056 to
    /// 12 over 4096 entries, which is a third of the table above white and an
    /// eighth below black.
    #[test]
    fn a_1d_table_keeps_its_values_past_the_ends() {
        let text = "LUT_1D_SIZE 3\n-0.05 0 0\n1 1 1\n12 12 12\n";
        let Lut::One(l) = Lut::from_cube(text).unwrap() else {
            panic!("1d");
        };
        assert_eq!(l.data[0], [-0.05, 1.0, 12.0]);
        assert_eq!(l.data[2], [0.0, 1.0, 12.0]);
        assert_eq!(l.sample(0, 0.5), 1.0);
        assert_eq!(l.sample(0, 1.0), 12.0);
        // Half-way between the last two nodes, and it stays above white.
        assert_eq!(l.sample(0, 0.75), 6.5);
        // The writer keeps both the values and the domain they were authored in.
        let back = Lut::One(l.clone());
        assert_eq!(
            Lut::from_cube(&back.to_cube()).unwrap(),
            back,
            "a 1D table drifts through a write/read cycle"
        );
    }

    /// A log-to-linear table is one entry per code of the *log* encoding, and
    /// cameras record at 10 and 12 bits: Apple's published Apple Log to Linear
    /// export lists 4096, and the density tables used for negative printing list
    /// 512 and 1024. A bound that keeps a 3D grid's memory sane says nothing
    /// about a one-dimensional list, whose length its own value lines settle
    /// anyway, so it has to be lifted for the 1D case. OpenColorIO draws the
    /// same line at different places: 129 for a grid side, 300 000 for a 1D
    /// table.
    #[test]
    fn a_long_1d_table_is_read_while_a_grid_keeps_its_bound() {
        // The shape of a log-to-linear curve: an eighth of a stop per entry
        // running from 2^-12 up to 2^2, so its own end sits well above white.
        let mut text = String::from("LUT_1D_SIZE 4096\n");
        for i in 0..4096 {
            let v = 2f32.powf(i as f32 / 4095.0 * 14.0 - 12.0);
            text.push_str(&format!("{v} {v} {v}\n"));
        }
        let Lut::One(l) = Lut::from_cube(&text).unwrap() else {
            panic!("4096 entries is a 1D table");
        };
        assert_eq!(l.len(), 4096);
        assert_eq!(Lut::One(l.clone()).size(), 4096);
        assert!(l.data[0][4095] > 3.9, "the superwhite end is the point");
        // A length the file declares but never fills is still refused.
        assert!(Lut::from_cube("LUT_1D_SIZE 4096\n0 0 0\n1 1 1\n").is_err());
        // Past the ceiling it is refused before its rows are read, and a grid
        // keeps the bound it always had.
        assert!(Lut::from_cube("LUT_1D_SIZE 300001\n0\n1\n").is_err());
        assert!(Lut::from_cube("LUT_3D_SIZE 200\n0 0 0\n1 1 1\n").is_err());
    }

    /// A `.cube` that is not a colour LUT — the extension is shared with
    /// Gaussian volumetric dumps and git-lfs pointers — has to be refused with a
    /// reason that fits, not one that names the other format. Every real colour
    /// table declares a size line, so a file without one is never a `.cube`, and
    /// the line that identifies it is the first one.
    #[test]
    fn a_cube_that_is_not_a_lut_says_what_it_is() {
        let gaussian = "Psi4 Gaussian Cube File.\n\n5 0.0 0.0 0.0\n1 0.1 0.0 0.0\n";
        let error = Lut::from_text(gaussian).unwrap_err().to_string();
        assert!(error.contains("not a LUT"), "{error}");
        assert!(error.contains(".cube"), "{error}");
        assert!(
            error.contains("\"Psi4 Gaussian Cube File.\""),
            "{error}: the line that gives the file away is not quoted"
        );
        // A size line still goes to the .cube reader and keeps its own errors.
        let short = Lut::from_text("LUT_3D_SIZE 2\n0 0 0\n1 1 1\n").unwrap_err();
        assert!(
            short.to_string().contains("3D LUT expected"),
            "{short}: a .cube must be judged as a .cube"
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
    fn every_reading_has_a_label_that_reads_back() {
        for mode in Interpolation::ALL {
            assert_eq!(Interpolation::from_label(mode.label()), Some(mode));
        }
        // `linear` is the word the tools outside fvid use for a trilinear read.
        assert_eq!(
            Interpolation::from_label("linear"),
            Some(Interpolation::Trilinear)
        );
        assert_eq!(Interpolation::from_label("bicubic"), None);
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
    fn every_camera_log_cube_can_stop_at_linear_light() {
        // A destination of `Transfer::Linear` writes the light itself: the curve
        // has no encoding, its `oetf` answers `None`, and the bake leaves the
        // value as it stands. Nothing else in the suite names that stop, so the
        // anchors here are the vendors' own code tables, read the way each table
        // is scaled — S-Log3 and Canon Log over 0..=1023, S-Log2/1 over the
        // 64..=940 legal range — and the expectation is scene reflectance rather
        // than a code for it.
        for (profile, code18, code90) in [
            (Log::SLog3, 420.0, 598.0),
            (Log::SLog2, 347.0, 582.0),
            (Log::SLog1, 394.0, 636.0),
            (Log::CLog, 351.0, 614.0),
        ] {
            let codes = profile.codes();
            let signal = |code: f32| (code - codes.black) / codes.span;
            let cube = CubePlan::camera_log(
                profile,
                Transfer::Linear,
                Primaries::BT709,
                Primaries::BT709,
                65,
            )
            .build();
            let grey = cube.sample([signal(code18); 3], Interpolation::Trilinear);
            let white = cube.sample([signal(code90); 3], Interpolation::Trilinear);
            // The residual is the grid and the reading between its nodes, not the
            // curve: at S-Log3's 90 % code the same probe reads 0.829 631 at 17
            // nodes, 0.903 043 at 65 and 0.901 261 at 129, closing on the tabled
            // 0.90 as the nodes multiply. The bounds below hold the measured worst
            // of the four curves at 65 nodes — 3.8e-4 on grey, 3.1e-3 on white,
            // the top of a log curve being where light-space nodes sit furthest
            // apart — with the headroom of one 8-bit code step (3.9e-3).
            assert!(close(grey[0], 0.18, 1e-3), "{profile:?} grey {grey:?}");
            assert!(close(white[0], 0.90, 4e-3), "{profile:?} white {white:?}");
            assert!(
                grey.iter().all(|v| (0.0..=1.0).contains(v)),
                "{profile:?} {grey:?}"
            );
        }
        // The stop is what separates the two answers: the same Sony 18 % code
        // measured as Rec.709 code for the same exposure is 0.409, not 0.18.
        let gamma = CubePlan::camera_log(
            Log::SLog3,
            Transfer::Bt709,
            Primaries::BT709,
            Primaries::BT709,
            65,
        )
        .build()
        .sample([420.0 / 1023.0; 3], Interpolation::Trilinear);
        assert!(close(gamma[0], 0.409_008, 2e-3), "{gamma:?}");
        // Above diffuse white a linear stop has nowhere to go, so the top of the
        // curve arrives at 1.0 and stays there — the price of asking for light.
        let linear = CubePlan::camera_log(
            Log::SLog3,
            Transfer::Linear,
            Primaries::BT709,
            Primaries::BT709,
            65,
        )
        .build();
        let top = linear.sample([1.0; 3], Interpolation::Trilinear);
        assert!(close(top[0], 1.0, 1e-4), "{top:?}");
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
        // Each node's row carries that node's own axis coordinates, so an entry
        // filed against the wrong axis comes back in the wrong channel.
        let mut text = String::from("3\n");
        for r in 0..3 {
            for g in 0..3 {
                for b in 0..3 {
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
        // The first row after the size line is the blue neighbour of black, not
        // the red one: a `.3dl` runs its grid blue-fastest, opposite to a cube.
        let blue = lut.sample([0.0, 0.0, 0.5], Interpolation::Nearest);
        assert!(
            blue[2] > 0.49 && blue[0] < 1e-3 && blue[1] < 1e-3,
            "the file's first row landed on the wrong axis: {blue:?}"
        );
        let red = lut.sample([1.0, 0.0, 0.0], Interpolation::Nearest);
        assert!(
            close(red[0], 1.0, 1e-3) && close(red[1], 0.0, 1e-6) && close(red[2], 0.0, 1e-3),
            "red came back as {red:?}"
        );
    }

    #[test]
    fn a_3dl_can_declare_its_grid_by_a_mesh_line_behind_keywords() {
        // Autodesk's export, Color Finesse and Photoshop's lookup plugin all
        // write a mesh line — one input code value per node along an axis, on
        // the 0…1023 scale — where a bare size would go, and Color Finesse
        // leads it with keyword lines that carry no node at all. This is the
        // 7-node shape of one of Photoshop's own files.
        const N: usize = 7;
        let mut sized = format!("{N}\n");
        let mut meshed = String::from(
            "#Do not edit\n\
             # LUT created by Synthetic Aperture Color Finesse 3 3.0.6(275)\n\
             \n\
             3DMESH\n\
             Mesh 4 12\n\
             0 171 341 512 682 853 1023\n",
        );
        for r in 0..N {
            for g in 0..N {
                for b in 0..N {
                    let row = format!("{} {} {}\n", r * 682, g * 682, b * 682);
                    sized.push_str(&row);
                    meshed.push_str(&row);
                }
            }
        }
        let a = Lut::from_3dl(&sized).unwrap();
        let b = Lut::from_3dl(&meshed).unwrap_or_else(|e| panic!("the mesh form: {e}"));
        assert_eq!(b.size(), N, "the mesh line declares the grid side");
        for rgb in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.5, 0.25, 0.75],
            [1.0, 1.0, 1.0],
        ] {
            for interp in [
                Interpolation::Nearest,
                Interpolation::Trilinear,
                Interpolation::Tetrahedral,
            ] {
                assert_eq!(
                    a.sample(rgb, interp),
                    b.sample(rgb, interp),
                    "{rgb:?} under {interp:?}: the header changed the look"
                );
            }
        }
        // The mesh line is not read as a row: the node red reaches is the one
        // the file's last row holds, not the ramp's top value.
        let red = b.sample([1.0, 0.0, 0.0], Interpolation::Nearest);
        assert!(close(red[0], 1.0, 1e-3), "{red:?}");
        assert!(
            close(red[1], 0.0, 1e-6) && close(red[2], 0.0, 1e-6),
            "{red:?}"
        );
    }

    /// One writer in the census states nothing: no size line and no mesh line,
    /// just the rows behind a comment and a `3DMESH` keyword. Their count is
    /// then the only declaration the file carries, so it is read as one — and
    /// only where it is an exact cube.
    #[test]
    fn a_3dl_with_no_declaration_is_read_from_its_row_count() {
        let mut text = String::new();
        for r in 0..3 {
            for g in 0..3 {
                for b in 0..3 {
                    text.push_str(&format!("{} {} {}\n", r * 2047, g * 2047, b * 2047));
                }
            }
        }
        let lut = Lut::from_3dl(&text).unwrap_or_else(|e| panic!("27 rows are a 3-grid: {e}"));
        assert_eq!(lut.size(), 3);
        // Read as a grid of the right width, the rows land on their own axes.
        let blue = lut.sample([0.0, 0.0, 1.0], Interpolation::Nearest);
        assert!(
            blue[2] > 0.99 && blue[0] < 1e-3,
            "the rows were filed against the wrong axis: {blue:?}"
        );
        // 28 rows are no one's grid.
        assert!(Lut::from_3dl(&format!("{text}0 0 0\n")).is_err());
    }

    /// One file of the 36 measured is written at 16 bits, and says so only in
    /// its header: `Mesh 4 16`, with nodes up to 65 535 where every other file
    /// stops at 4095. The second number is the depth the format's own writers
    /// put there, so it is the divisor the codes mean to be read at. The
    /// instrument is the real file's own shape — an identity grid, whose every
    /// node stores its own coordinates — because nothing else in the format
    /// states the scale it was written at: read at the declared depth it comes
    /// back as the ramp, and at any other it does not exist at all.
    #[test]
    fn a_3dl_divides_by_the_depth_its_header_declares() {
        const N: usize = 7;
        let code = |i: usize| (i as f32 * 65535.0 / (N - 1) as f32).round() as u32;
        let mut body = String::new();
        for r in 0..N {
            for g in 0..N {
                for b in 0..N {
                    body.push_str(&format!("{} {} {}\n", code(r), code(g), code(b)));
                }
            }
        }
        let lut = Lut::from_3dl(&format!("3DMESH\nMesh 4 16\n{N}\n{body}"))
            .expect("a 16-bit .3dl is read");
        assert_eq!(lut.size(), N);
        let mut worst = 0.0f32;
        for r in 0..N {
            for g in 0..N {
                for b in 0..N {
                    let rgb = [
                        r as f32 / (N - 1) as f32,
                        g as f32 / (N - 1) as f32,
                        b as f32 / (N - 1) as f32,
                    ];
                    let out = lut.sample(rgb, Interpolation::Nearest);
                    for ch in 0..3 {
                        worst = worst.max((out[ch] - rgb[ch]).abs());
                    }
                }
            }
        }
        assert!(
            worst <= 1.0 / 65535.0 + 1e-7,
            "the declared depth left the ramp by {worst}, over a code of {}",
            1.0 / 65535.0
        );
        // The same rows at the 12-bit divisor are a wall of white: everything
        // but black clips, which is what makes reading the header matter.
        let blind = Lut::from_3dl(&format!("3DMESH\nMesh 4 12\n{N}\n{body}")).unwrap();
        let mut saturated = 0usize;
        for i in 0..N * N * N {
            let rgb = [
                (i / (N * N)) as f32 / (N - 1) as f32,
                (i / N % N) as f32 / (N - 1) as f32,
                (i % N) as f32 / (N - 1) as f32,
            ];
            let out = blind.sample(rgb, Interpolation::Nearest);
            saturated += usize::from(out.iter().any(|v| *v >= 0.999_9));
        }
        assert_eq!(
            saturated,
            N * N * N - 1,
            "at the wrong divisor all but black should clip"
        );
    }

    #[test]
    fn bad_3dl_files_are_rejected() {
        assert!(Lut::from_3dl("3\n0 0 0\n").is_err());
        // Two rows with nothing to say how wide the grid is: not a cube.
        assert!(Lut::from_3dl("0 0 0\n1 1 1\n").is_err());
        assert!(Lut::from_3dl("3\n0 0\n").is_err());
        // A line of four or more values is a mesh declaration only before the
        // grid starts; once rows are coming in it is a row of the wrong width.
        assert!(
            Lut::from_3dl("3DMESH\n0 171 341 512 682 853 1023\n0 0 0\n0 0 0 0\n").is_err(),
            "a wide row after the grid starts"
        );
        // A mesh line that does not match the rows behind it is not a size.
        assert!(Lut::from_3dl("0 171 341 512 682 853 1023\n0 0 0\n").is_err());
        // Neither line says how wide the grid is.
        assert!(Lut::from_3dl("3\n0 0 0\n3\n1 1 1\n").is_err());
    }

    /// Which axis a `.dat` varies fastest is settled against `ffmpeg` by the
    /// fixture test in `tests/lut_ffmpeg.rs`. What is held here is the shape of
    /// the grid on a table whose every row carries its own node's coordinates, so
    /// a node filed against the wrong axis comes back in the wrong channel.
    #[test]
    fn a_dat_lists_its_grid_blue_fastest_like_a_3dl() {
        let mut text = String::from("3DLUTSIZE 3\n");
        let mut rows: Vec<[f32; 3]> = Vec::new();
        for r in 0..3 {
            for g in 0..3 {
                for b in 0..3 {
                    let row = [r as f32 / 2.0, g as f32 / 2.0, b as f32 / 2.0];
                    text.push_str(&format!("{} {} {}\n", row[0], row[1], row[2]));
                    rows.push(row);
                }
            }
        }
        let lut = Lut::from_dat(&text).unwrap();
        assert_eq!(lut.size(), 3);
        // The line straight after the directive is the blue neighbour of black
        // rather than the red one, and red is nine lines in — one for each node
        // along blue.
        let blue = lut.sample([0.0, 0.0, 0.5], Interpolation::Nearest);
        assert!(
            blue[2] > 0.49 && blue[0] < 1e-6 && blue[1] < 1e-6,
            "the file's first row landed on the wrong axis: {blue:?}"
        );
        let red = lut.sample([1.0, 0.0, 0.0], Interpolation::Nearest);
        assert!(
            red[0] > 0.99 && red[1] < 1e-6 && red[2] < 1e-6,
            "red came back as {red:?}"
        );
        // The identical rows taken in the order a `.cube` lists them are a
        // different look, so the transposition is not a matter of tidiness.
        let blind = Lut::Three(Lut3d {
            size: 3,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            data: rows,
        });
        let moved = blind.sample([1.0, 0.0, 0.0], Interpolation::Nearest);
        assert!(
            moved[0] < 1e-6 && moved[2] > 0.99,
            "a `.dat` read as a `.cube` puts red on blue: {moved:?}"
        );
    }

    /// `ffmpeg` drops blank and `#` lines anywhere in a `.dat`, and with no
    /// `3DLUTSIZE` line in front it fixes the grid at 33 nodes and reads the rows
    /// that follow — so a headerless 8-line file is an unexpected EOF there, not
    /// a 2-grid. Here the row count declares the grid as it does for a `.3dl`
    /// that states nothing, and the comments do not count towards it.
    #[test]
    fn a_dat_without_a_directive_is_read_from_its_row_count_past_comments() {
        let commented = "\
# an Iridas look
3DLUTSIZE 2
0 0 0

0 0 1
0 1 0
0 1 1
1 0 0
1 0 1
1 1 0
# a note before the last corner
1 1 1
";
        let lut = Lut::from_dat(commented).unwrap();
        assert_eq!(lut.size(), 2);
        assert_eq!(
            Lut::from_text(commented).unwrap(),
            lut,
            "a comment in front of the directive moved the file to another reader"
        );
        let headerless = "\
0 0 0
0 0 1
0 1 0
0 1 1
1 0 0
1 0 1
1 1 0
1 1 1
";
        assert_eq!(
            Lut::from_text(headerless).unwrap(),
            Lut::from_dat(headerless).unwrap()
        );
    }

    /// A `.dat` carries fractions of white and a `.3dl` carries codes up to
    /// 2^bits−1, which is the only thing telling the two apart when neither
    /// states its grid — so the sniff has to be one the codes cannot pass, and
    /// each reader keeps its own scale.
    #[test]
    fn a_dat_of_fractions_and_a_3dl_of_codes_do_not_change_places() {
        let codes = "\
0 0 0
0 0 4095
0 4095 0
0 4095 4095
4095 0 0
4095 0 4095
4095 4095 0
4095 4095 4095
";
        let lut = Lut::from_text(codes).unwrap();
        assert!(
            matches!(lut, Lut::Three(_)),
            "a headerless grid of codes up to 4 095 is a .3dl"
        );
        let blue = lut.sample([0.0, 0.0, 1.0], Interpolation::Nearest);
        assert!(close(blue[2], 1.0, 1e-6), "{blue:?}");

        let fractions = codes.replace("4095", "1");
        let as_dat = Lut::from_text(&fractions).unwrap();
        let unscaled = as_dat.sample([0.0, 0.0, 1.0], Interpolation::Nearest);
        assert!(close(unscaled[2], 1.0, 1e-6), "{unscaled:?}");
        // The same rows taken by the other reader are a picture at nothing: the
        // one scale that separates the formats is the one that matters.
        let as_3dl = Lut::from_3dl(&fractions).unwrap();
        let divided = as_3dl.sample([0.0, 0.0, 1.0], Interpolation::Nearest);
        assert!(
            close(divided[2], 1.0 / 4095.0, 1e-6),
            "{divided:?} against {unscaled:?}"
        );
    }

    /// The nodes are the look, unscaled and unclipped: a `.dat` that runs below
    /// black or above white carries that as written, the way a `.cube` does, so
    /// the interpolators can bend the shoulder instead of flattening it.
    #[test]
    fn a_dat_keeps_its_nodes_unscaled_and_unclipped() {
        let text = "\
3DLUTSIZE 2
-0.05 -0.05 -0.05
0 0 0
0 0 0
0 0 0
0 0 0
0 0 0
0 0 0
1.2 1.2 1.2
";
        let lut = Lut::from_dat(text).unwrap();
        let black = lut.sample([0.0, 0.0, 0.0], Interpolation::Nearest);
        assert!(close(black[0], -0.05, 1e-6), "{black:?}");
        let white = lut.sample([1.0, 1.0, 1.0], Interpolation::Nearest);
        assert!(close(white[0], 1.2, 1e-6), "{white:?}");
    }

    #[test]
    fn bad_dat_files_are_rejected() {
        // Nothing at all, and a directive with nothing to say.
        assert!(Lut::from_dat("").is_err());
        assert!(Lut::from_dat("3DLUTSIZE\n0 0 0\n").is_err());
        assert!(Lut::from_dat("3DLUTSIZE x\n0 0 0\n").is_err());
        // A grid of one node has nothing to interpolate, and the bound the
        // sampler is sized for is the one the grid formats share.
        assert!(Lut::from_dat("3DLUTSIZE 1\n0 0 0\n").is_err());
        assert!(Lut::from_dat(&format!("3DLUTSIZE {}\n0 0 0\n", MAX_3D_SIDE + 1)).is_err());
        // One directive, and only before the rows begin.
        assert!(Lut::from_dat("0 0 0\n3DLUTSIZE 2\n").is_err());
        assert!(Lut::from_dat("3DLUTSIZE 2\n3DLUTSIZE 2\n").is_err());
        // A row is three numbers: a fourth is not ignored, and neither is a
        // word or an infinite value.
        assert!(Lut::from_dat("3DLUTSIZE 2\n0 0 0 0\n").is_err());
        assert!(Lut::from_dat("3DLUTSIZE 2\n0 0\n").is_err());
        assert!(Lut::from_dat("3DLUTSIZE 2\n0 0 off\n").is_err());
        assert!(Lut::from_dat("3DLUTSIZE 2\n0 0 inf\n").is_err());
        // The rows have to make the grid the file names.
        assert!(Lut::from_dat("3DLUTSIZE 2\n0 0 0\n").is_err());
        let mut one_row_short = String::from("3DLUTSIZE 2\n");
        for i in 0..7 {
            one_row_short.push_str(&format!("0 0 {i}\n"));
        }
        assert!(Lut::from_dat(&one_row_short).is_err());
        // With no directive, a count that is not an exact cube declares nothing.
        assert!(Lut::from_dat("0 0 0\n1 1 1\n").is_err());
    }

    const SPI1D_CURVE: &str = "\
Version 1
From -0.125 1.125
Length 3
Components 1
{
        -0.0096749
        0.5
        1.3083107
}
";

    const SPI1D_SRGB: &str = include_str!("../../tests/fixtures/lut/sRGB_to_linear.spi1d");
    const SPI3D_BIZARRE: &str = include_str!("../../tests/fixtures/lut/lut3d_bizarre.spi3d");

    /// A `.spi1d` states the signal range once, for all three channels, as
    /// `From`, and the row count is the table's length — the index is the
    /// position in the file, so there is nothing in a row to place it.
    #[test]
    fn a_spi1d_curve_is_read_with_its_from_pair_as_the_input_range() {
        let Lut::One(l) = &Lut::from_spi1d(SPI1D_CURVE).unwrap() else {
            panic!("a .spi1d is a 1D table");
        };
        assert_eq!(l.len(), 3);
        assert_eq!(l.domain_min, [-0.125; 3]);
        assert_eq!(l.domain_max, [1.125; 3]);
        // One value per row is the same curve on all three channels.
        assert_eq!(l.data[0], l.data[1]);
        assert_eq!(l.data[1], l.data[2]);
        // The declared ends are what the sampler reads them as, and the values
        // beyond them are kept: a curve that runs past the ends is the signal.
        assert!(close(l.sample(0, -0.125), -0.009_674_9, 1e-6));
        assert!(close(l.sample(0, 0.5), 0.5, 1e-6));
        assert!(close(l.sample(0, 1.125), 1.308_310_7, 1e-6));
        assert!(l.data[0][0] < 0.0 && l.data[0][2] > 1.0);
    }

    #[test]
    fn a_spi1d_spreads_one_two_or_three_values_per_row() {
        let Lut::One(l) =
            &Lut::from_spi1d("Version 1\nLength 2\nComponents 1\n{\n0.2\n0.8\n}\n").unwrap()
        else {
            panic!("1D");
        };
        assert_eq!(l.data, [vec![0.2f32, 0.8], vec![0.2, 0.8], vec![0.2, 0.8]]);
        // No `From` tag is the display range, which is the same default a .cube
        // carries when it states no domain.
        assert_eq!(l.domain_min, [0.0; 3]);
        assert_eq!(l.domain_max, [1.0; 3]);
        // Two values state red and green and leave blue at nothing; three are
        // red, green, blue. These are the reference reader's rules, and a table
        // that got them the other way round would grade blue to a constant.
        let Lut::One(two) =
            &Lut::from_spi1d("Version 1\nLength 2\nComponents 2\n{\n0.2 0.4\n0.6 0.8\n}\n")
                .unwrap()
        else {
            panic!("1D");
        };
        assert_eq!(
            two.data,
            [vec![0.2f32, 0.6], vec![0.4, 0.8], vec![0.0, 0.0]]
        );
        let Lut::One(three) =
            &Lut::from_spi1d("Version 1\nLength 2\nComponents 3\n{\n0.2 0.4 0.6\n0.8 0.9 1.0\n}\n")
                .unwrap()
        else {
            panic!("1D");
        };
        assert_eq!(
            three.data,
            [vec![0.2f32, 0.8], vec![0.4, 0.9], vec![0.6, 1.0]]
        );
    }

    /// The table OpenColorIO ships as its sRGB shaper, checked against fvid's own
    /// sRGB curve. This is the check that the `From` pair is an input range and
    /// not a scale on the output: at −0.125 the index it declares is 0 and the
    /// value there is not zero, and at 1.0 the index is 3690 and the value is one.
    ///
    /// FFmpeg has no reader for this format, so the second answer comes from
    /// `ociochecklut` of OpenColorIO 2.5.2, which reads the same file and returns
    /// 0.050 876 09 at 0.25, 0.214 041 1 at 0.5 and 0.522 521 6 at 0.75 — the sRGB
    /// decode of each input, which is what the probes below assert.
    #[test]
    fn a_real_spi1d_is_the_srgb_curve_over_the_range_it_declares() {
        let lut = Lut::from_text(SPI1D_SRGB).expect("the OpenColorIO table reads");
        let Lut::One(l) = &lut else {
            panic!("a .spi1d is a 1D table");
        };
        assert_eq!(l.len(), 4101);
        assert_eq!(l.domain_min, [-0.125; 3]);
        assert_eq!(l.domain_max, [1.125; 3]);
        for i in (410..=3690).step_by(97) {
            let signal = -0.125f32 + i as f32 * (1.25 / 4100.0);
            let want = Transfer::Srgb.eotf(signal).unwrap();
            assert!(
                close(l.data[0][i], want, 1e-5),
                "entry {i} at signal {signal}: {} against {want}",
                l.data[0][i]
            );
        }
        // Past the ends the curve keeps running, which is the headroom a shaper
        // space is there to carry.
        assert!(close(l.data[0][0], -0.009_674_9, 1e-5));
        assert!(close(l.data[0][4100], 1.308_31, 1e-5));
        // Sampling is what a grade does with the table, so the declared range has
        // to reach the sampler at its ends and in its middle.
        assert!(close(l.sample(0, -0.125), -0.009_674_9, 1e-5));
        assert!(close(l.sample(2, 1.0), 1.0, 1e-5));
        assert!(close(
            l.sample(1, 0.5),
            Transfer::Srgb.eotf(0.5).unwrap(),
            1e-5
        ));
    }

    /// The grid OpenColorIO's own test suite uses to catch a reader that assumes a
    /// row order: its rows run with blue fastest, which is the reverse of how
    /// fvid's table stores a node, and every row states the node it means.
    ///
    /// FFmpeg refuses this file, so the corroborating reader is `ociochecklut` of
    /// OpenColorIO 2.5.2 again, and it answers at the corners the row order
    /// decides: 1.622 678 red at [1, 0, 0], 0.097 751 71 green at [0, 0, 1],
    /// 0.371 456 5 red at [0.5, 0.5, 0.5]. The same digits the three asserts below
    /// read off the parsed grid.
    #[test]
    fn a_spi3d_row_places_its_node_by_its_own_indices() {
        let lut = Lut::from_spi3d(SPI3D_BIZARRE).unwrap();
        assert_eq!(lut.size(), 3);
        let Lut::Three(l) = &lut else {
            panic!("a .spi3d is a grid");
        };
        assert_eq!(l.domain_min, [0.0; 3]);
        assert_eq!(l.domain_max, [1.0; 3]);
        // (2, 0, 0) is the eleventh row of the file; read by position it would
        // land in slot 10 of a table whose slot 2 is red at full scale.
        assert!(close(l.at(2, 0, 0)[0], 1.622_678_4, 1e-6));
        assert!(close(l.at(0, 0, 2)[1], 0.097_751_71, 1e-6));
        assert!(close(l.at(1, 1, 1)[0], 0.371_456_5, 1e-6));
        assert!(close(l.at(2, 2, 2)[2], 1.173_020_5, 1e-6));
        // Overshoot is kept as authored, the same way a `.cube` keeps it.
        assert!(l.at(0, 2, 0)[1] > 1.4);
        assert!(l.at(0, 0, 0)[0] < 0.0);
        // The rows in reverse order cannot change the table, because the order is
        // not what says where a node goes.
        let mut rows: Vec<&str> = SPI3D_BIZARRE
            .lines()
            .skip(3)
            .filter(|line| !line.trim().is_empty())
            .collect();
        assert_eq!(rows.len(), 27);
        rows.reverse();
        let shuffled = format!("SPILUT 1.0\n3 3\n3 3 3\n{}\n", rows.join("\n"));
        assert_eq!(Lut::from_spi3d(&shuffled).unwrap(), lut);
    }

    /// The two readers of the same numbers must not disagree: the grid written out
    /// as a `.cube`, red fastest, reads back as the same table.
    #[test]
    fn a_spi3d_and_the_cube_of_the_same_numbers_agree() {
        let spi = Lut::from_spi3d(SPI3D_BIZARRE).unwrap();
        let cube = Lut::from_cube(&spi.to_cube()).unwrap();
        assert_eq!(cube, spi);
        for mode in Interpolation::ALL {
            for i in 0..=8 {
                let rgb = [i as f32 / 8.0, 1.0 - i as f32 / 8.0, (i % 5) as f32 / 4.0];
                assert_eq!(
                    cube.sample(rgb, mode),
                    spi.sample(rgb, mode),
                    "{mode:?} {rgb:?}"
                );
            }
        }
    }

    #[test]
    fn bad_spi_files_are_rejected() {
        fn spi3d(rows: &[&str]) -> String {
            format!("SPILUT 1.0\n2 2\n2 2 2\n{}\n", rows.join("\n"))
        }
        const NODES: [(u32, u32, u32); 8] = [
            (0, 0, 0),
            (0, 0, 1),
            (0, 1, 0),
            (0, 1, 1),
            (1, 0, 0),
            (1, 0, 1),
            (1, 1, 0),
            (1, 1, 1),
        ];
        let rows: Vec<String> = NODES
            .iter()
            .map(|(r, g, b)| format!("{r} {g} {b} 0 0 0"))
            .collect();
        let all: Vec<&str> = rows.iter().map(String::as_str).collect();
        // The positive control first: all eight nodes is a grid.
        assert!(Lut::from_text(&spi3d(&all)).is_ok());
        let mut seven = all.clone();
        seven[7] = "2 0 0 0 0 0";
        // Eight rows, one index twice and one node never: the count is right, so
        // only the duplicate check can refuse this.
        let mut dup = all.clone();
        dup[7] = all[0];
        let mut wide = all.clone();
        wide[7] = "1 1 1 0 0";
        let mut prose = all.clone();
        prose[7] = "1 1 1 0 0 x";
        for bad in [
            // A grid with most of its nodes missing is not a smaller grid.
            spi3d(&all[..1]),
            spi3d(&dup),
            // An index the declared grid has no node at.
            spi3d(&seven),
            // A row that is not six fields, and a value that is not a number.
            spi3d(&wide),
            spi3d(&prose),
            "SPILUT 1.0\n2 2\n3 3 3\n0 0 0 0 0 0\n".to_string(),
            "SPILUT 1.0\n2 2\n4 4 2\n".to_string(),
            "SPILUT 1.0\n2 2\n1 1 1\n".to_string(),
            "SPILUT 1.0\n2 2\n130 130 130\n".to_string(),
            "SPILUT 1.0\n2 2\n2 2\n".to_string(),
            "COLOURLUT 1.0\n2 2\n2 2 2\n".to_string(),
            String::new(),
            // The four tags a .spi1d has to state.
            "Length 2\nComponents 1\n{\n0\n1\n}\n".to_string(),
            "Version 1\nComponents 1\n{\n0\n1\n}\n".to_string(),
            "Version 1\nLength 2\n{\n0\n1\n}\n".to_string(),
            "Version 1\nLength 2\nComponents 0\n{\n\n}\n".to_string(),
            "Version 1\nLength 2\nComponents 4\n{\n0 0 0 0\n1 1 1 1\n}\n".to_string(),
            "Version 1\nLength 1\nComponents 1\n{\n0\n}\n".to_string(),
            // Short, and over: the count is a declaration, not a guess.
            "Version 1\nLength 4\nComponents 1\n{\n0\n1\n}\n".to_string(),
            "Version 1\nLength 2\nComponents 1\n{\n0\n1\n2\n}\n".to_string(),
            // A row of the width the table does not declare, and a value that is
            // not a number.
            "Version 1\nLength 2\nComponents 2\n{\n0 0 0\n1 1 1\n}\n".to_string(),
            "Version 1\nLength 2\nComponents 1\n{\n0\nx\n}\n".to_string(),
            // A table that never opens its brace holds no rows at all.
            "Version 1\nLength 2\nComponents 1\n{\n0\n".to_string(),
            // A row ahead of the tag that says how wide a row is.
            "Version 1\nLength 2\n{\n0\n1\n}\n".to_string(),
        ] {
            assert!(Lut::from_text(&bad).is_err(), "accepted: {bad:?}");
        }
    }

    /// Every one of the five formats is reached from the content alone, which is
    /// how `--lut` takes a path, and a file that is none of them says so about all
    /// five.
    #[test]
    fn from_text_routes_each_format_to_its_own_reader() {
        assert!(matches!(Lut::from_text(SPI1D_CURVE).unwrap(), Lut::One(_)));
        assert!(matches!(
            Lut::from_text(SPI3D_BIZARRE).unwrap(),
            Lut::Three(_)
        ));
        assert!(matches!(Lut::from_text(CUBE_2).unwrap(), Lut::Three(_)));
        let three_dl = "3DMESH\nMesh 4 12\n2\n0 0 0\n4095 0 0\n0 4095 0\n4095 4095 0\n0 0 4095\n4095 0 4095\n0 4095 4095\n4095 4095 4095\n";
        assert!(Lut::from_text(three_dl).is_ok());
        // One `3DLUTSIZE` line at the head is all a `.dat` needs to be recognised,
        // and it is read as a grid of nodes rather than a table of codes.
        let dat = "3DLUTSIZE 2\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";
        assert!(Lut::from_text(dat).is_ok());
        // A grid that stops short is judged as a .spi3d, not as a .3dl whose
        // header lines happen to look like numbers.
        let short = Lut::from_text("SPILUT 1.0\n3 3\n3 3 3\n0 0 0 0 0 0\n").unwrap_err();
        assert!(short.to_string().contains(".spi3d"), "{short}");
        let short = Lut::from_text("Version 1\nLength 4\nComponents 1\n{\n0\n}\n").unwrap_err();
        assert!(short.to_string().contains(".spi1d"), "{short}");
        let err = Lut::from_text("Psi4 Gaussian Cube File.\n\n5 0.0 0.0 0.0\n")
            .unwrap_err()
            .to_string();
        for name in [".cube", ".spi1d", ".spi3d", ".dat", ".3dl"] {
            assert!(err.contains(name), "{err}: {name} is not named");
        }
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

    /// A conversion that only changes the triangle leaves every grey exactly
    /// where it is — both spaces state the same white, so a node on the diagonal
    /// maps to itself through XYZ and back. A check that reads the diagonal
    /// therefore calls such a grid no change at all, and a grade built on one is
    /// skipped even though every saturated colour moves: measured here, the
    /// camera's own red leaves the diagonal by more than two codes a channel
    /// while seventeen greys come back untouched.
    #[test]
    fn a_grid_that_only_moves_the_triangle_is_not_an_identity() {
        let tol = 0.5 / 255.0;
        let lut = Lut::Three(
            CubePlan::transfer(
                Transfer::Bt709,
                Transfer::Bt709,
                Primaries::S_GAMUT3_CINE,
                Primaries::BT709,
                33,
            )
            .build(),
        );
        for i in 0..=16 {
            let v = i as f32 / 16.0;
            let out = lut.sample([v, v, v], Interpolation::Tetrahedral);
            assert!(
                out.iter().zip([v, v, v]).all(|(a, b)| (a - b).abs() <= tol),
                "grey {v} came out {out:?}"
            );
        }
        for corner in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            let out = lut.sample(corner, Interpolation::Trilinear);
            assert!(
                out.iter()
                    .zip(corner)
                    .any(|(a, b)| (a - b).abs() > 8.0 * tol),
                "{corner:?} came out {out:?}"
            );
        }
        assert!(!lut.is_identity(tol), "the triangle moved");
    }

    /// A cube with no tone map changes the curve and nothing else, which means
    /// the light a code states survives: BT.2100's two families read the same
    /// code as ten times as many cd/m² apart, and an unmapped conversion that
    /// ignored that made a code carrying 1 000 cd/m² leave as a dark grey.
    #[test]
    fn a_plain_transfer_cube_keeps_the_light_its_codes_state() {
        let cube = CubePlan::transfer(
            Transfer::Pq,
            Transfer::Srgb,
            Primaries::BT2020,
            Primaries::BT709,
            33,
        )
        .build();
        assert_eq!(cube.size, 33);
        // The destination is an SDR curve with no panel of its own, so its
        // scale is the reference white BT.2100 prints for one.
        let white = crate::color::transfer::SDR_PEAK_NITS;
        // Up to the destination's own white. The last stop short of it, since a
        // 33-node grid interpolates the codes either side of the clip and cannot
        // land on it from between them.
        for nits in [5.0, 20.0, 100.0] {
            let code = Transfer::Pq.from_nits(nits, white).unwrap();
            let out = cube.sample([code; 3], Interpolation::Tetrahedral);
            for channel in out {
                let back = Transfer::Srgb.to_nits(channel, white).unwrap();
                assert!(
                    close(back, nits, nits * 0.03),
                    "{nits} cd/m² (PQ {code:.4}) came out {out:?}"
                );
            }
        }
        assert!(close(
            cube.sample([0.0; 3], Interpolation::Trilinear)[0],
            0.0,
            1e-6
        ));
        // Past the destination's own white there is no code left to hold the
        // light, so PQ's whole 10 000-nit top lands on the one code there is.
        assert!(close(
            cube.sample([1.0; 3], Interpolation::Trilinear)[0],
            1.0,
            1e-3
        ));
    }

    /// The same rule where the two families disagree by an order of magnitude:
    /// SDR's 1.0 is a panel's white and PQ's is 10 000 cd/m², so carrying one
    /// into the other has to move the numbers or the picture changes brightness.
    #[test]
    fn an_unmapped_conversion_moves_between_curve_families() {
        let white = crate::color::transfer::SDR_PEAK_NITS;
        let up = CubePlan::transfer(
            Transfer::Bt709,
            Transfer::Pq,
            Primaries::BT709,
            Primaries::BT2020,
            33,
        )
        .build();
        // Diffuse white leaves as the code that states it, not as the top of a
        // 10 000-nit scale.
        let out = up.sample([1.0; 3], Interpolation::Trilinear)[0];
        let nits = Transfer::Pq.to_nits(out, white).unwrap();
        assert!(close(nits, white, 2.0), "SDR white left as {nits} cd/m²");
        // A quarter of the panel's white rides the same ratio down the scale.
        let out = up.sample([0.5; 3], Interpolation::Trilinear)[0];
        let nits = Transfer::Pq.to_nits(out, white).unwrap();
        let video = Transfer::Bt709.eotf(0.5).unwrap() * white;
        assert!(
            close(nits, video, video * 0.02),
            "code 0.5 states {video} cd/m², left as {nits}"
        );
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
