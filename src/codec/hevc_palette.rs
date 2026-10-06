//! Bounded SCC palette predictor state (H.265 8.4.4, equation 8-79).
use crate::{Result, invalid};

pub(crate) struct Header {
    pub(crate) entries: Vec<[u16; 3]>,
    pub(crate) reused: Vec<bool>,
    pub(crate) escapes: bool,
    pub(crate) transpose: bool,
    indices: Vec<u8>,
    final_copy: bool,
    maximum: u8,
}
impl Header {
    /// Parse syntax before delta_qp()/chroma_qp_offset(). The CU owner handles
    /// those shared QP syntax elements before continuing with samples().
    pub(crate) fn read(
        predictor: &Predictor,
        size: usize,
        b: &mut super::hevc_cabac::HevcCabac<'_>,
    ) -> Result<Self> {
        use super::hevc_cabac::Syntax;
        let (entries, reused) = predictor.read_entries(|| b.bypass())?;
        let escapes = entries.is_empty() || b.bypass()?;
        let maximum = (entries.len() + usize::from(escapes) - 1) as u8;
        let indices = read_indices(size, maximum, || b.bypass())?;
        let (final_copy, transpose) = if maximum > 0 {
            (
                b.decision(Syntax::PaletteCopyAbove, 0)?,
                b.decision(Syntax::PaletteTranspose, 0)?,
            )
        } else {
            (false, false)
        };
        Ok(Self {
            entries,
            reused,
            escapes,
            transpose,
            indices,
            final_copy,
            maximum,
        })
    }
    pub(crate) fn samples(
        &self,
        size: usize,
        chroma: u8,
        depths: [u8; 2],
        bypass: bool,
        qp: [i32; 3],
        b: &mut super::hevc_cabac::HevcCabac<'_>,
    ) -> Result<[Vec<u16>; 3]> {
        use super::hevc_cabac::Syntax;
        let map = read_map(
            size,
            self.maximum,
            &self.indices,
            self.final_copy,
            |event| match event {
                MapSyntax::CopyAbove => b.decision(Syntax::PaletteCopyAbove, 0).map(usize::from),
                MapSyntax::Run {
                    maximum,
                    copy_above,
                    index_idc,
                } => read_run(maximum, copy_above, index_idc, |context| match context {
                    Some(c) => b.decision(Syntax::PaletteRun, c),
                    None => b.bypass(),
                }),
            },
        )?;
        let escapes = if self.escapes {
            Some(read_escapes(
                size,
                chroma,
                depths,
                self.transpose,
                self.maximum,
                &map,
                bypass,
                || b.bypass(),
            )?)
        } else {
            None
        };
        reconstruct(
            size,
            chroma,
            depths,
            self.transpose,
            &self.entries,
            &map,
            escapes.as_deref(),
            bypass,
            qp,
        )
    }
}

/// Palette escapes use FL(bitDepth) in bypass CUs, EG3 otherwise (9.3.3.12).
pub(crate) fn read_escape(
    depth: u8,
    bypass: bool,
    mut bit: impl FnMut() -> Result<bool>,
) -> Result<u32> {
    if !(8..=16).contains(&depth) {
        return Err(invalid("invalid HEVC palette escape depth"));
    }
    let mut order = if bypass { u32::from(depth) } else { 3 };
    let mut value = 0u64;
    if !bypass {
        while bit()? {
            if order >= 32 {
                return Err(invalid("HEVC palette escape exceeds integer range"));
            }
            value += 1u64 << order;
            order += 1;
        }
    }
    let mut suffix = 0u64;
    for _ in 0..order {
        suffix = (suffix << 1) | u64::from(bit()?);
    }
    u32::try_from(value + suffix).map_err(|_| invalid("HEVC palette escape exceeds integer range"))
}

/// Read component-major escape samples in traversal coordinates, including the
/// transposed 4:2:2 sampling condition in palette_coding().
pub(crate) fn read_escapes(
    size: usize,
    chroma: u8,
    depths: [u8; 2],
    transpose: bool,
    maximum_index: u8,
    map: &[u8],
    bypass: bool,
    mut bit: impl FnMut() -> Result<bool>,
) -> Result<Vec<[u32; 3]>> {
    if !matches!(size, 4 | 8 | 16 | 32)
        || chroma > 3
        || maximum_index > 64
        || map.len() != size * size
        || map.iter().any(|&i| i > maximum_index)
        || depths.iter().any(|d| !(8..=16).contains(d))
    {
        return Err(invalid("invalid HEVC palette escape map"));
    }
    let mut output = vec![[0; 3]; map.len()];
    for c in 0..if chroma == 0 { 1 } else { 3 } {
        for position in 0..map.len() {
            let y = position / size;
            let column = position % size;
            let x = if y % 2 == 0 {
                column
            } else {
                size - 1 - column
            };
            let at = y * size + x;
            let present = c == 0
                || match chroma {
                    1 => x % 2 == 0 && y % 2 == 0,
                    2 => {
                        if transpose {
                            y % 2 == 0
                        } else {
                            x % 2 == 0
                        }
                    }
                    3 => true,
                    _ => false,
                };
            if map[at] == maximum_index && present {
                output[at][c] = read_escape(depths[usize::from(c != 0)], bypass, &mut bit)?;
            }
        }
    }
    Ok(output)
}

/// Bypass-coded index count (Rice/EGk) and index sequence (9.3.3.13/14).
pub(crate) fn read_indices(
    size: usize,
    maximum_index: u8,
    mut bit: impl FnMut() -> Result<bool>,
) -> Result<Vec<u8>> {
    if !matches!(size, 4 | 8 | 16 | 32) || maximum_index > 64 {
        return Err(invalid("invalid HEVC palette index geometry"));
    }
    if maximum_index == 0 {
        return Ok(vec![0]);
    }
    let rice = 3 + ((usize::from(maximum_index) + 1) >> 3);
    let mut prefix = 0;
    while prefix < 4 && bit()? {
        prefix += 1;
    }
    let mut value = prefix << rice;
    if prefix < 4 {
        let mut remainder = 0;
        for _ in 0..rice {
            remainder = (remainder << 1) | usize::from(bit()?);
        }
        value += remainder;
    } else {
        let mut order = rice + 1;
        while bit()? {
            if order > 10 {
                return Err(invalid("HEVC palette index count exceeds block"));
            }
            value += 1usize << order;
            order += 1;
        }
        let mut suffix = 0;
        for _ in 0..order {
            suffix = (suffix << 1) | usize::from(bit()?);
        }
        value += suffix;
    }
    if value >= size * size {
        return Err(invalid("HEVC palette index count exceeds block"));
    }
    let mut indices = Vec::with_capacity(value + 1);
    for index in 0..=value {
        let maximum = usize::from(maximum_index) - usize::from(index != 0);
        if maximum == 0 {
            indices.push(0);
            continue;
        }
        let width = (usize::BITS - maximum.leading_zeros()) as usize;
        let threshold = (1usize << width) - (maximum + 1);
        let mut v = 0;
        for _ in 0..width - 1 {
            v = (v << 1) | usize::from(bit()?);
        }
        if v >= threshold {
            v = (v << 1) + usize::from(bit()?) - threshold;
        }
        indices.push(v as u8);
    }
    Ok(indices)
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MapSyntax {
    CopyAbove,
    Run {
        maximum: usize,
        copy_above: bool,
        index_idc: u8,
    },
}

/// Expand the complete palette_coding() map, including inferred copy/run modes
/// and the omitted reference index (7-83/84). Transposition is applied later.
pub(crate) fn read_map(
    size: usize,
    maximum_index: u8,
    indices: &[u8],
    final_copy: bool,
    mut syntax: impl FnMut(MapSyntax) -> Result<usize>,
) -> Result<Vec<u8>> {
    if !matches!(size, 4 | 8 | 16 | 32)
        || maximum_index > 64
        || indices.is_empty()
        || indices.len() > size * size
        || indices[0] > maximum_index
        || indices.iter().skip(1).any(|&i| i >= maximum_index)
    {
        return Err(invalid("invalid HEVC palette index sequence"));
    }
    if maximum_index == 0 {
        if indices != [0] || final_copy {
            return Err(invalid("invalid HEVC single-index palette map"));
        }
        return Ok(vec![0; size * size]);
    }
    let at = |position: usize| {
        let y = position / size;
        let x = position % size;
        y * size + if y % 2 == 0 { x } else { size - 1 - x }
    };
    let mut map = vec![0; size * size];
    let mut copies = vec![false; size * size];
    let mut position = 0;
    let mut consumed = 0;
    while position < map.len() {
        let remaining = indices.len() - consumed;
        let current = at(position);
        let previous = position.checked_sub(1).map(at);
        let copy = if position >= size && previous.is_some_and(|p| !copies[p]) {
            if remaining > 0 && position < map.len() - 1 {
                match syntax(MapSyntax::CopyAbove)? {
                    0 => false,
                    1 => true,
                    _ => return Err(invalid("invalid HEVC palette copy flag")),
                }
            } else {
                !(position == map.len() - 1 && remaining > 0)
            }
        } else {
            false
        };
        let mut idc = 0;
        let index = if copy {
            0
        } else {
            idc = *indices
                .get(consumed)
                .ok_or_else(|| invalid("HEVC palette indices exhausted"))?;
            consumed += 1;
            let reference = previous.map_or(usize::from(maximum_index) + 1, |p| {
                if copies[p] {
                    usize::from(map[current - size])
                } else {
                    usize::from(map[p])
                }
            });
            let value = usize::from(idc) + usize::from(usize::from(idc) >= reference);
            if value > usize::from(maximum_index) {
                return Err(invalid("HEVC adjusted palette index exceeds maximum"));
            }
            value as u8
        };
        let remaining = indices.len() - consumed;
        let length = if remaining > 0 || copy != final_copy {
            let maximum = (map.len() - position - 1)
                .checked_sub(remaining + usize::from(final_copy))
                .ok_or_else(|| invalid("HEVC palette indices leave no room for runs"))?;
            let run = if maximum == 0 {
                0
            } else {
                syntax(MapSyntax::Run {
                    maximum,
                    copy_above: copy,
                    index_idc: idc,
                })?
            };
            if run > maximum {
                return Err(invalid("HEVC palette run exceeds derived maximum"));
            }
            run + 1
        } else {
            map.len() - position
        };
        for _ in 0..length {
            let p = at(position);
            copies[p] = copy;
            map[p] = if copy { map[p - size] } else { index };
            position += 1;
        }
    }
    if consumed != indices.len() {
        return Err(invalid("unused HEVC palette indices"));
    }
    Ok(map)
}

/// Table 9-51; bins after index four use the bypass engine.
pub(crate) fn run_context(copy_above: bool, palette_index: u8, bin: usize) -> Option<usize> {
    if bin > 4 {
        return None;
    }
    if copy_above {
        Some([5, 6, 6, 7, 7][bin])
    } else if bin == 0 {
        Some(if palette_index < 1 {
            0
        } else if palette_index < 3 {
            1
        } else {
            2
        })
    } else {
        Some([0, 3, 3, 4, 4][bin])
    }
}

/// TR prefix and TB suffix from Table 9-32 and equations 7-85..86.
pub(crate) fn read_run(
    maximum: usize,
    copy_above: bool,
    palette_index: u8,
    mut bit: impl FnMut(Option<usize>) -> Result<bool>,
) -> Result<usize> {
    if maximum > 1023 {
        return Err(invalid("HEVC palette run maximum exceeds block"));
    }
    if maximum == 0 {
        return Ok(0);
    }
    let prefix_max = (usize::BITS - maximum.leading_zeros()) as usize;
    let mut prefix = 0;
    while prefix < prefix_max && bit(run_context(copy_above, palette_index, prefix))? {
        prefix += 1;
    }
    if prefix < 2 {
        return Ok(prefix);
    }
    let offset = 1usize << (prefix - 1);
    let suffix_max = if offset * 2 > maximum {
        maximum - offset
    } else {
        offset - 1
    };
    if suffix_max == 0 {
        return Ok(offset);
    }
    let width = (usize::BITS - suffix_max.leading_zeros()) as usize;
    let threshold = (1usize << width) - (suffix_max + 1);
    let mut suffix = 0;
    for _ in 0..width - 1 {
        suffix = (suffix << 1) | usize::from(bit(None)?);
    }
    if suffix >= threshold {
        suffix = (suffix << 1) + usize::from(bit(None)?) - threshold;
    }
    Ok(offset + suffix)
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum RunKind {
    Index(u8),
    CopyAbove,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Run {
    pub(crate) kind: RunKind,
    pub(crate) length: usize,
}

/// Expand decoded runs in the normative alternating-row traversal (6-14).
pub(crate) fn expand_runs(size: usize, maximum_index: u8, runs: &[Run]) -> Result<Vec<u8>> {
    if !matches!(size, 4 | 8 | 16 | 32) || maximum_index > 64 {
        return Err(invalid("invalid HEVC palette run geometry"));
    }
    let mut map = vec![0; size * size];
    let mut position = 0;
    for run in runs {
        if run.length == 0 || run.length > map.len() - position {
            return Err(invalid("HEVC palette run exceeds block"));
        }
        if let RunKind::Index(index) = run.kind {
            if index > maximum_index {
                return Err(invalid("HEVC palette run index exceeds maximum"));
            }
        } else if position < size {
            return Err(invalid("HEVC palette copy-above starts on first row"));
        }
        for _ in 0..run.length {
            let y = position / size;
            let column = position % size;
            let x = if y % 2 == 0 {
                column
            } else {
                size - 1 - column
            };
            let at = y * size + x;
            map[at] = match run.kind {
                RunKind::Index(i) => i,
                RunKind::CopyAbove => map[at - size],
            };
            position += 1;
        }
    }
    if position != map.len() {
        return Err(invalid("HEVC palette runs do not cover block"));
    }
    Ok(map)
}

/// Reconstruct palette samples from the scan-domain index/escape maps (8-69..78).
pub(crate) fn reconstruct(
    size: usize,
    chroma: u8,
    depths: [u8; 2],
    transpose: bool,
    entries: &[[u16; 3]],
    indices: &[u8],
    escapes: Option<&[[u32; 3]]>,
    bypass: bool,
    qp: [i32; 3],
) -> Result<[Vec<u16>; 3]> {
    if !matches!(size, 4 | 8 | 16 | 32)
        || chroma > 3
        || depths.iter().any(|d| !(8..=16).contains(d))
        || entries.len() > 64
        || indices.len() != size * size
        || escapes.is_some_and(|e| e.len() != size * size)
        || qp.iter().any(|&v| !(-48..=99).contains(&v))
    {
        return Err(invalid("invalid HEVC palette reconstruction geometry"));
    }
    let components = if chroma == 0 { 1 } else { 3 };
    for entry in entries {
        for c in 0..components {
            if u32::from(entry[c]) >= 1u32 << depths[usize::from(c != 0)] {
                return Err(invalid("HEVC palette sample exceeds component depth"));
            }
        }
    }
    // Validate the full luma map even where subsampled chroma skips locations.
    if indices.iter().any(|&i| {
        usize::from(i) > entries.len() || (usize::from(i) == entries.len() && escapes.is_none())
    }) {
        return Err(invalid("HEVC palette index leaves current palette"));
    }
    let mut output: [Vec<u16>; 3] = std::array::from_fn(|_| Vec::new());
    for c in 0..components {
        let [sx, sy] = if c == 0 || chroma == 3 {
            [1, 1]
        } else if chroma == 1 {
            [2, 2]
        } else {
            [2, 1]
        };
        let width = size / sx;
        let height = size / sy;
        output[c].reserve(width * height);
        for y in 0..height {
            for x in 0..width {
                let (xx, yy) = if transpose {
                    (y * sy, x * sx)
                } else {
                    (x * sx, y * sy)
                };
                let at = yy * size + xx;
                let index = usize::from(indices[at]);
                let maximum = (1u32 << depths[usize::from(c != 0)]) - 1;
                let sample = if index < entries.len() {
                    entries[index][c]
                } else {
                    let escape = escapes.unwrap()[at][c];
                    if bypass {
                        if escape > maximum {
                            return Err(invalid("HEVC palette escape exceeds component depth"));
                        }
                        escape as u16
                    } else {
                        let q = qp[c].max(0) as u32;
                        let scale = [40i64, 45, 51, 57, 64, 72][(q % 6) as usize];
                        let sample = ((i64::from(escape) * scale) << (q / 6)) + 32;
                        (sample >> 6).clamp(0, i64::from(maximum)) as u16
                    }
                };
                output[c].push(sample);
            }
        }
    }
    Ok(output)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Predictor {
    entries: Vec<[u16; 3]>,
    maximum: usize,
    palette_maximum: usize,
    depths: [u8; 2],
    components: usize,
}
impl Predictor {
    pub(crate) fn from_parameters(
        sps: &super::hevc_sps::Sps,
        pps: &super::hevc_pps::Pps,
    ) -> Result<Self> {
        let palette = sps
            .palette
            .as_ref()
            .ok_or_else(|| invalid("HEVC palette predictor requires SPS capability"))?;
        let initial = pps.palette_initial.as_deref().unwrap_or(&palette.initial);
        Self::new(
            usize::from(palette.maximum),
            usize::from(palette.predictor_maximum),
            sps.depth,
            if sps.chroma_format == 0 { 1 } else { 3 },
            initial,
        )
    }
    /// Decode bypass-coded reuse runs and new samples from palette_coding().
    /// State changes only after the caller reconstructs and accepts the CU.
    pub(crate) fn read_entries(
        &self,
        mut bypass: impl FnMut() -> Result<bool>,
    ) -> Result<(Vec<[u16; 3]>, Vec<bool>)> {
        fn eg0(bits: &mut impl FnMut() -> Result<bool>) -> Result<usize> {
            let mut prefix = 0;
            while bits()? {
                prefix += 1;
                if prefix > 7 {
                    return Err(invalid("HEVC palette run exceeds bounded range"));
                }
            }
            let mut suffix = 0;
            for _ in 0..prefix {
                suffix = (suffix << 1) | usize::from(bits()?);
            }
            Ok((1usize << prefix) - 1 + suffix)
        }
        let mut reused = vec![false; self.entries.len()];
        let mut current = Vec::with_capacity(self.palette_maximum);
        let mut index = 0;
        while index < self.entries.len() && current.len() < self.palette_maximum {
            let run = eg0(&mut bypass)?;
            if run == 1 {
                break;
            }
            if run > 1 {
                index += run - 1;
            }
            if index >= self.entries.len() {
                return Err(invalid("HEVC palette reuse run leaves predictor"));
            }
            reused[index] = true;
            current.push(self.entries[index]);
            index += 1;
        }
        let count = if current.len() < self.palette_maximum {
            eg0(&mut bypass)?
        } else {
            0
        };
        if count > self.palette_maximum - current.len() {
            return Err(invalid("HEVC palette entry count exceeds maximum"));
        }
        let start = current.len();
        current.resize(start + count, [0; 3]);
        for c in 0..self.components {
            for entry in &mut current[start..] {
                let mut sample = 0u16;
                for _ in 0..self.depths[usize::from(c != 0)] {
                    sample = (sample << 1) | u16::from(bypass()?);
                }
                entry[c] = sample;
            }
        }
        Ok((current, reused))
    }
    pub(crate) fn new(
        palette_maximum: usize,
        maximum: usize,
        depths: [u8; 2],
        components: usize,
        initial: &[[u16; 3]],
    ) -> Result<Self> {
        if palette_maximum > 64
            || maximum > 128
            || maximum < palette_maximum
            || (palette_maximum == 0 && maximum != 0)
            || !matches!(components, 1 | 3)
            || depths.iter().any(|d| !(8..=16).contains(d))
            || initial.len() > maximum
        {
            return Err(invalid("invalid HEVC palette predictor parameters"));
        }
        let state = Self {
            entries: initial.to_vec(),
            maximum,
            palette_maximum,
            depths,
            components,
        };
        state.validate_samples(initial)?;
        Ok(state)
    }
    fn validate_samples(&self, entries: &[[u16; 3]]) -> Result<()> {
        for entry in entries {
            for (c, &sample) in entry.iter().enumerate().take(self.components) {
                if u32::from(sample) >= (1u32 << self.depths[usize::from(c != 0)]) {
                    return Err(invalid("HEVC palette sample exceeds component depth"));
                }
            }
        }
        Ok(())
    }
    /// Current entries include reused predictor entries first, in predictor order.
    /// Build the next predictor transactionally so a malformed CU cannot poison it.
    pub(crate) fn update(&mut self, current: &[[u16; 3]], reused: &[bool]) -> Result<()> {
        if current.len() > self.palette_maximum || reused.len() != self.entries.len() {
            return Err(invalid("invalid HEVC palette predictor update size"));
        }
        self.validate_samples(current)?;
        let selected: Vec<_> = self
            .entries
            .iter()
            .zip(reused)
            .filter_map(|(e, r)| r.then_some(*e))
            .collect();
        if !current.starts_with(&selected) {
            return Err(invalid("HEVC palette reuse disagrees with current entries"));
        }
        let mut next = Vec::with_capacity(self.maximum);
        next.extend_from_slice(current);
        for (entry, &used) in self.entries.iter().zip(reused) {
            if next.len() == self.maximum {
                break;
            }
            if !used {
                next.push(*entry);
            }
        }
        self.entries = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameter_initializers_inherit_or_override_as_specified() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 10] {
            for name in ["intra", "initializers", "pps-initializers"] {
                let data =
                    std::fs::read(root.join(format!("hevc-scc-palette-{name}-rext{depth}.mp4")))
                        .unwrap();
                let input = crate::container::mp4::Mp4Reader::open(
                    std::io::Cursor::new(data),
                    Default::default(),
                )
                .unwrap();
                let config =
                    super::super::config::HevcConfig::parse(&input.tracks()[0].configuration)
                        .unwrap();
                let sps = super::super::hevc_sps::Sps::parse(
                    config
                        .arrays
                        .iter()
                        .find(|a| a.nal_type == 33)
                        .unwrap()
                        .units[0],
                    16 << 20,
                )
                .unwrap();
                let mut pps = super::super::hevc_pps::Pps::parse(
                    config
                        .arrays
                        .iter()
                        .find(|a| a.nal_type == 34)
                        .unwrap()
                        .units[0],
                    &sps,
                    16 << 20,
                )
                .unwrap();
                let initial = Predictor::from_parameters(&sps, &pps).unwrap();
                assert_eq!(
                    initial.entries,
                    pps.palette_initial
                        .as_ref()
                        .unwrap_or(&sps.palette.as_ref().unwrap().initial)
                        .clone()
                );
                pps.palette_initial = Some(vec![]);
                assert!(
                    Predictor::from_parameters(&sps, &pps)
                        .unwrap()
                        .entries
                        .is_empty()
                );
                pps.palette_initial = Some(vec![[1, 2, 3]]);
                assert_eq!(
                    Predictor::from_parameters(&sps, &pps).unwrap().entries,
                    [[1, 2, 3]]
                );
            }
        }
    }
    #[test]
    fn cabac_bridge_reconstructs_empty_palette_escape_block() {
        use super::super::hevc_cabac::{HevcCabac, SliceType};
        let predictor = Predictor::new(0, 0, [8; 2], 3, &[]).unwrap();
        let mut b = HevcCabac::new(&[0; 2048], 0, SliceType::I, false, 22).unwrap();
        let header = Header::read(&predictor, 8, &mut b).unwrap();
        assert!(header.entries.is_empty() && header.reused.is_empty());
        assert!(header.escapes);
        assert!(!header.transpose);
        let samples = header
            .samples(8, 3, [8; 2], false, [22; 3], &mut b)
            .unwrap();
        assert_eq!(samples, [vec![0; 64], vec![0; 64], vec![0; 64]]);
    }
    #[test]
    fn escape_fixed_lengths_and_eg3_are_bounded() {
        for (expected, bits) in [
            (0, "0000"),
            (7, "0111"),
            (8, "100000"),
            (23, "101111"),
            (24, "11000000"),
        ] {
            let mut input = bits.bytes();
            assert_eq!(
                read_escape(8, false, || input
                    .next()
                    .map(|b| b == b'1')
                    .ok_or_else(|| invalid("truncated escape")))
                .unwrap(),
                expected
            );
            assert!(input.next().is_none());
        }
        for depth in 8..=16 {
            let mut read = 0;
            assert_eq!(
                read_escape(depth, true, || {
                    read += 1;
                    Ok(true)
                })
                .unwrap(),
                (1u32 << depth) - 1
            );
            assert_eq!(read, depth);
        }
        assert!(read_escape(8, false, || Ok(true)).is_err());
        assert!(read_escape(8, true, || Err(invalid("truncated escape"))).is_err());
    }
    #[test]
    fn escape_map_consumes_only_subsampled_component_locations() {
        for chroma in 0..=3 {
            for transpose in [false, true] {
                let mut bits = 0;
                let escapes =
                    read_escapes(8, chroma, [8, 10], transpose, 0, &[0; 64], true, || {
                        bits += 1;
                        Ok(true)
                    })
                    .unwrap();
                let chroma_samples = match chroma {
                    0 => 0,
                    1 => 16,
                    2 => 32,
                    _ => 64,
                };
                assert_eq!(bits, 64 * 8 + 2 * chroma_samples * 10);
                let p = reconstruct(
                    8,
                    chroma,
                    [8, 10],
                    transpose,
                    &[],
                    &[0; 64],
                    Some(&escapes),
                    true,
                    [0; 3],
                )
                .unwrap();
                assert_eq!(p[0], [255; 64]);
                assert_eq!(p[1], vec![1023; chroma_samples]);
                assert_eq!(p[2], vec![1023; chroma_samples]);
            }
        }
        let mut consumed = false;
        assert!(
            read_escapes(8, 3, [8; 2], false, 1, &[2; 64], true, || {
                consumed = true;
                Ok(false)
            })
            .is_err()
        );
        assert!(!consumed);
    }
    #[test]
    fn index_sequence_uses_rice_count_and_omits_later_binary_index() {
        let mut bits = "00011".bytes();
        assert_eq!(
            read_indices(8, 1, || bits
                .next()
                .map(|b| b == b'1')
                .ok_or_else(|| invalid("truncated index")))
            .unwrap(),
            [1, 0]
        );
        assert!(bits.next().is_none());
        assert_eq!(
            read_indices(8, 0, || panic!("inferred indices")).unwrap(),
            [0]
        );
        assert!(read_indices(8, 1, || Ok(true)).is_err());
        assert!(read_indices(8, 1, || Err(invalid("truncated index"))).is_err());
    }
    #[test]
    fn complete_map_adjusts_indices_and_infers_final_copy() {
        let mut events = 0;
        let map = read_map(8, 2, &[1, 1], true, |event| {
            events += 1;
            match event {
                MapSyntax::Run {
                    maximum: 61,
                    copy_above: false,
                    index_idc: 1,
                } => Ok(3),
                MapSyntax::Run {
                    maximum: 58,
                    copy_above: false,
                    index_idc: 1,
                } => Ok(3),
                _ => Err(invalid("unexpected palette syntax")),
            }
        })
        .unwrap();
        assert_eq!(events, 2);
        for row in map.chunks_exact(8) {
            assert_eq!(row, [1, 1, 1, 1, 2, 2, 2, 2]);
        }
        assert_eq!(
            read_map(4, 0, &[0], false, |_| panic!("single index is inferred")).unwrap(),
            [0; 16]
        );
        assert!(read_map(8, 1, &[0, 1], false, |_| Ok(0)).is_err());
        assert!(read_map(8, 2, &[0, 0], false, |_| Ok(usize::MAX)).is_err());
    }
    #[test]
    fn truncated_prefix_and_binary_suffix_cover_run_boundaries() {
        for (value, bits) in [
            (0, "0"),
            (1, "10"),
            (2, "1100"),
            (3, "1101"),
            (4, "1110"),
            (5, "1111"),
        ] {
            let mut input = bits.bytes();
            assert_eq!(
                read_run(5, false, 0, |_| input
                    .next()
                    .map(|b| b == b'1')
                    .ok_or_else(|| invalid("truncated run")))
                .unwrap(),
                value
            );
            assert!(input.next().is_none());
        }
        assert_eq!(
            read_run(0, true, 0, |_| panic!("inferred run consumes no bits")).unwrap(),
            0
        );
        assert_eq!(read_run(1, true, 0, |_| Ok(true)).unwrap(), 1);
        assert!(read_run(1024, false, 0, |_| Ok(false)).is_err());
        assert!(read_run(5, false, 0, |_| Err(invalid("truncated run"))).is_err());
    }
    #[test]
    fn run_prefix_contexts_switch_to_bypass_after_fifth_bin() {
        for index in 0..=64 {
            assert_eq!(
                (0..6)
                    .map(|b| run_context(true, index, b))
                    .collect::<Vec<_>>(),
                [Some(5), Some(6), Some(6), Some(7), Some(7), None]
            );
            let first = if index == 0 {
                0
            } else if index < 3 {
                1
            } else {
                2
            };
            assert_eq!(
                (0..6)
                    .map(|b| run_context(false, index, b))
                    .collect::<Vec<_>>(),
                [Some(first), Some(3), Some(3), Some(4), Some(4), None]
            );
        }
        assert_eq!(run_context(true, 0, usize::MAX), None);
    }
    #[test]
    fn alternating_scan_and_copy_above_cross_row_boundaries() {
        let mut runs: Vec<_> = (0..8)
            .map(|i| Run {
                kind: RunKind::Index(i),
                length: 1,
            })
            .collect();
        runs.push(Run {
            kind: RunKind::CopyAbove,
            length: 56,
        });
        let map = expand_runs(8, 7, &runs).unwrap();
        for row in map.chunks_exact(8) {
            assert_eq!(row, &[0, 1, 2, 3, 4, 5, 6, 7]);
        }
        let map = expand_runs(
            4,
            1,
            &[
                Run {
                    kind: RunKind::Index(0),
                    length: 5,
                },
                Run {
                    kind: RunKind::Index(1),
                    length: 11,
                },
            ],
        )
        .unwrap();
        assert_eq!(&map[4..8], &[1, 1, 1, 0]);
        for runs in [
            vec![Run {
                kind: RunKind::CopyAbove,
                length: 64,
            }],
            vec![Run {
                kind: RunKind::Index(0),
                length: 65,
            }],
            vec![Run {
                kind: RunKind::Index(0),
                length: 0,
            }],
            vec![Run {
                kind: RunKind::Index(2),
                length: 64,
            }],
            vec![Run {
                kind: RunKind::Index(0),
                length: 63,
            }],
        ] {
            assert!(expand_runs(8, 1, &runs).is_err());
        }
    }
    #[test]
    fn reconstructed_maps_transpose_and_subsample_in_luma_coordinates() {
        let entries = [[11, 21, 31], [12, 22, 32]];
        let map: Vec<_> = (0..64)
            .map(|i| u8::from(i % 8 == 2 && i / 8 == 4))
            .collect();
        for chroma in 0..=3 {
            for transpose in [false, true] {
                let p = reconstruct(
                    8, chroma, [8; 2], transpose, &entries, &map, None, true, [0; 3],
                )
                .unwrap();
                let (x, y) = if transpose { (4, 2) } else { (2, 4) };
                assert_eq!(p[0][y * 8 + x], 12);
                if chroma != 0 {
                    let sx = if chroma == 3 { 1 } else { 2 };
                    let sy = if chroma == 1 { 2 } else { 1 };
                    assert_eq!(p[1][(y / sy) * (8 / sx) + x / sx], 22);
                    assert_eq!(p[2][(y / sy) * (8 / sx) + x / sx], 32);
                } else {
                    assert!(p[1].is_empty() && p[2].is_empty());
                }
            }
        }
    }
    #[test]
    fn escape_scaling_clips_and_bypass_rejects_out_of_depth() {
        let map = [0; 64];
        let escapes = [[4, 1023, u32::MAX]; 64];
        let p = reconstruct(
            8,
            3,
            [8, 10],
            false,
            &[],
            &map,
            Some(&escapes),
            false,
            [0, 6, 99],
        )
        .unwrap();
        assert_eq!(p[0], [3; 64]);
        assert_eq!(p[1], [1023; 64]);
        assert_eq!(p[2], [1023; 64]);
        assert!(
            reconstruct(
                8,
                3,
                [8, 10],
                false,
                &[],
                &map,
                Some(&escapes),
                true,
                [0; 3]
            )
            .is_err()
        );
        assert!(reconstruct(8, 0, [8; 2], false, &[], &map, None, true, [0; 3]).is_err());
        assert!(reconstruct(8, 0, [8; 2], false, &[[1; 3]], &[2; 64], None, true, [0; 3]).is_err());
        assert!(reconstruct(usize::MAX, 3, [8; 2], false, &[], &[], None, true, [0; 3]).is_err());
    }
    #[test]
    fn bypass_runs_and_component_major_samples_are_bounded() {
        fn reader(bits: &str) -> impl FnMut() -> Result<bool> + '_ {
            let mut values = bits.bytes();
            move || {
                values
                    .next()
                    .map(|b| b == b'1')
                    .ok_or_else(|| invalid("truncated palette syntax"))
            }
        }
        let state = Predictor::new(3, 4, [8; 2], 3, &[[1, 2, 3], [4, 5, 6], [7, 8, 9]]).unwrap();
        // EG0(2) skips first entry, EG0(1) terminates; EG0(1) adds a color.
        let (entries, reused) = state
            .read_entries(reader(concat!(
                "101", "100", "100", "00000001", "00000010", "00000011"
            )))
            .unwrap();
        assert_eq!(reused, [false, true, false]);
        assert_eq!(entries, [[4, 5, 6], [1, 2, 3]]);
        assert!(state.read_entries(reader("11111111")).is_err());
        assert!(state.read_entries(reader("11011")).is_err());
        assert!(state.read_entries(reader("1011001")).is_err());
    }
    #[test]
    fn reused_order_and_unused_tail_follow_normative_update() {
        let a = [1, 2, 3];
        let b = [4, 5, 6];
        let c = [7, 8, 9];
        let d = [10, 11, 12];
        let mut state = Predictor::new(3, 4, [8; 2], 3, &[a, b, c]).unwrap();
        state.update(&[b, d], &[false, true, false]).unwrap();
        assert_eq!(state.entries, [b, d, a, c]);
        state
            .update(&[c, a, d], &[false, true, true, true])
            .unwrap_err();
        state
            .update(&[d, a, c], &[false, true, true, true])
            .unwrap();
        assert_eq!(state.entries, [d, a, c, b]);
        state.update(&[], &[false; 4]).unwrap();
        assert_eq!(state.entries, [d, a, c, b]);
    }
    #[test]
    fn malformed_updates_are_atomic_and_depth_is_component_specific() {
        let mut state = Predictor::new(2, 3, [8, 10], 3, &[[255, 1023, 1000]]).unwrap();
        let before = state.clone();
        for (entries, reused) in [
            (vec![[256, 0, 0]], vec![false]),
            (vec![[0, 1024, 0]], vec![false]),
            (vec![[1, 2, 3]], vec![true]),
            (vec![[0; 3]; 3], vec![false]),
            (vec![], vec![]),
        ] {
            assert!(state.update(&entries, &reused).is_err());
            assert_eq!(state, before);
        }
        assert!(Predictor::new(0, 1, [8; 2], 3, &[]).is_err());
        assert!(Predictor::new(65, 128, [8; 2], 3, &[]).is_err());
        assert!(Predictor::new(64, 129, [8; 2], 3, &[]).is_err());
        let mono = Predictor::new(1, 1, [16; 2], 1, &[[65535, 65535, 65535]]).unwrap();
        assert_eq!(mono.entries.len(), 1);
    }
}
