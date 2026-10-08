//! AV1 frame header syntax with explicit reference metadata.
use super::{
    av1_sequence::Sequence,
    av1_tiles::{Layout, align, signed},
    bits::BitReader,
};
use crate::{Result, invalid};
#[derive(Clone, Debug)]
pub struct Quantization {
    pub base: u8,
    pub delta: [i32; 5], // Y DC, U DC/AC, V DC/AC
    pub matrix: Option<[u8; 3]>,
    pub delta_resolution: Option<u8>,
}
#[derive(Clone, Debug)]
pub struct LoopFilter {
    pub levels: [u8; 4],
    pub sharpness: u8,
    pub deltas_enabled: bool,
    pub reference_deltas: [i32; 8],
    pub mode_deltas: [i32; 2],
    pub delta_resolution: Option<(u8, bool)>,
}
#[derive(Clone, Debug)]
pub struct Cdef {
    pub damping: u8,
    pub bits: u8,
    pub strengths: [[u8; 4]; 8],
}
#[derive(Clone, Debug)]
pub struct Header {
    pub grain: Option<Grain>,
    pub frame_type: u8,
    pub frame_id: Option<u32>,
    pub show: bool,
    pub showable: bool,
    pub error_resilient: bool,
    pub disable_cdf_update: bool,
    pub disable_frame_end_update: bool,
    pub screen_content: bool,
    pub intrabc: bool,
    pub order_hint: u32,
    pub refresh_flags: u8,
    /// Existing reference slots invalidated by error-resilient order hints.
    pub invalidated_references: u8,
    pub size_override: bool,
    pub size: [u32; 2],
    pub render_size: [u32; 2],
    pub upscaled_width: u32,
    pub superres_denom: u8,
    pub tiles: Layout,
    pub quant: Quantization,
    pub segments: [[Option<i32>; 8]; 8],
    pub segmentation_enabled: bool,
    pub segmentation_update_map: bool,
    pub segmentation_temporal_update: bool,
    pub segmentation_update_data: bool,
    pub lossless: [bool; 8],
    pub filter: LoopFilter,
    pub cdef: Cdef,
    /// Semantic restoration enum: NONE=0, WIENER=1, SGRPROJ=2, SWITCHABLE=3.
    pub restoration_types: [u8; 3],
    pub restoration_sizes: [u32; 3],
    /// 0 = 4x4 only, 1 = largest, 2 = selected per block.
    pub tx_mode: u8,
    pub reduced_tx_set: bool,
    pub header_bytes: usize,
    pub primary_reference: usize,
    pub references: [usize; 7],
    pub integer_mv: bool,
    pub high_precision_mv: bool,
    pub interpolation_filter: usize,
    pub motion_mode_switchable: bool,
    pub reference_mvs: bool,
    pub reference_select: bool,
    pub skip_mode: Option<[usize; 2]>,
    pub warped_motion: bool,
    pub global_types: [u8; 7],
    pub global_params: [[i64; 6]; 7],
}
#[path = "av1_grain_params.rs"]
mod grain_params;
pub use grain_params::Grain;

const IDENTITY_GLOBAL: [i64; 6] = [0, 0, 65536, 0, 0, 65536];
fn global_unsigned(b: &mut BitReader<'_>, n: u32) -> Result<u32> {
    if n == 1 {
        return Ok(0);
    }
    let w = n.ilog2() + 1;
    let m = (1 << w) - n;
    let v = b.read((w - 1) as u8)?;
    if v < m {
        Ok(v)
    } else {
        Ok((v << 1) - m + b.read(1)?)
    }
}
fn global_parameter(b: &mut BitReader<'_>, low: i64, high: i64, reference: i64) -> Result<i64> {
    let n = (high - low) as u32;
    let mut i = 0;
    let mut offset = 0;
    let value = loop {
        let bits = if i == 0 { 3 } else { i + 2 };
        let a = 1u32 << bits;
        if n <= offset + 3 * a {
            break offset + global_unsigned(b, n - offset)?;
        }
        if !b.bit()? {
            break offset + b.read(bits as u8)?;
        }
        offset += a;
        i += 1;
    } as i64;
    let r = reference - low;
    if !(0..high - low).contains(&r) {
        return Err(invalid("invalid AV1 global motion reference parameter"));
    }
    let recenter = |r: i64, v: i64| {
        if v > 2 * r {
            v
        } else if v & 1 != 0 {
            r - (v + 1) / 2
        } else {
            r + v / 2
        }
    };
    Ok(low
        + if 2 * r <= i64::from(n) {
            recenter(r, value)
        } else {
            i64::from(n) - 1 - recenter(i64::from(n) - 1 - r, value)
        })
}
/// AV1 set_frame_refs: ties follow the normative slot scan order.
fn short_references(
    current: u32,
    bits: u8,
    last: usize,
    golden: usize,
    hints: [u32; 8],
) -> Result<[usize; 7]> {
    let midpoint = 1i32 << (bits - 1);
    let mask = (1i32 << bits) - 1;
    let shifted = hints.map(|hint| {
        let distance = (hint.wrapping_sub(current) as i32) & mask;
        midpoint + ((distance & (midpoint - 1)) - (distance & midpoint))
    });
    if shifted[last] >= midpoint || shifted[golden] >= midpoint {
        return Err(invalid("AV1 short LAST/GOLDEN reference is not forward"));
    }
    let mut references = [usize::MAX; 7];
    references[0] = last;
    references[3] = golden;
    let mut used = [false; 8];
    used[last] = true;
    used[golden] = true;
    for (target, latest) in [(6, true), (4, false), (5, false)] {
        let candidates = (0..8).filter(|&i| !used[i] && shifted[i] >= midpoint);
        let selected = if latest {
            candidates.max_by_key(|&i| (shifted[i], i))
        } else {
            candidates.min_by_key(|&i| (shifted[i], i))
        };
        if let Some(index) = selected {
            references[target] = index;
            used[index] = true;
        }
    }
    for target in [1, 2, 4, 5, 6] {
        if references[target] == usize::MAX {
            if let Some(index) = (0..8)
                .filter(|&i| !used[i] && shifted[i] < midpoint)
                .max_by_key(|&i| (shifted[i], i))
            {
                references[target] = index;
                used[index] = true;
            }
        }
    }
    let earliest = (0..8).min_by_key(|&i| (shifted[i], i)).unwrap();
    for index in &mut references {
        if *index == usize::MAX {
            *index = earliest;
        }
    }
    Ok(references)
}
fn delta_q(b: &mut BitReader<'_>) -> Result<i32> {
    if b.bit()? { signed(b, 7) } else { Ok(0) }
}
impl Header {
    /// Parse an intra frame in an OBU_FRAME payload (zero byte alignment).
    pub fn parse_intra(s: &Sequence, data: &[u8], temporal_id: u8, spatial_id: u8) -> Result<Self> {
        let header = Self::parse(s, data, temporal_id, spatial_id, &[None; 8])?;
        if header.frame_type != 0 && header.frame_type != 2 {
            return Err(invalid("expected AV1 intra frame"));
        }
        Ok(header)
    }
    pub fn parse(
        s: &Sequence,
        data: &[u8],
        temporal_id: u8,
        spatial_id: u8,
        refs: &[Option<&Header>; 8],
    ) -> Result<Self> {
        Self::parse_with_order_hints(
            s,
            data,
            temporal_id,
            spatial_id,
            refs,
            refs.map(|reference| reference.map_or(0, |header| header.order_hint)),
        )
    }
    pub(crate) fn parse_with_order_hints(
        s: &Sequence,
        data: &[u8],
        temporal_id: u8,
        spatial_id: u8,
        refs: &[Option<&Header>; 8],
        reference_order_hints: [u32; 8],
    ) -> Result<Self> {
        Self::parse_framing(
            s,
            data,
            temporal_id,
            spatial_id,
            refs,
            reference_order_hints,
            false,
        )
    }
    pub(crate) fn parse_separate(
        s: &Sequence,
        data: &[u8],
        temporal_id: u8,
        spatial_id: u8,
        refs: &[Option<&Header>; 8],
        reference_order_hints: [u32; 8],
    ) -> Result<Self> {
        Self::parse_framing(
            s,
            data,
            temporal_id,
            spatial_id,
            refs,
            reference_order_hints,
            true,
        )
    }
    fn parse_framing(
        s: &Sequence,
        data: &[u8],
        temporal_id: u8,
        spatial_id: u8,
        refs: &[Option<&Header>; 8],
        reference_order_hints: [u32; 8],
        separate: bool,
    ) -> Result<Self> {
        let mut adjusted_refs = *refs;
        let mut invalidated_references = 0u8;
        let b = &mut BitReader::new(data);
        let (frame_type, show, showable, error_resilient) = if s.reduced_header {
            (0, true, false, true)
        } else {
            if b.bit()? {
                return Err(invalid("AV1 show-existing-frame requires reference state"));
            }
            let kind = b.read(2)? as u8;
            let show = b.bit()?;
            if show
                && s.timing
                    .as_ref()
                    .is_none_or(|t| t.ticks_per_picture.is_none())
            {
                if let Some(model) = &s.decoder_model {
                    b.read(model.presentation_bits)?;
                }
            }
            let showable = if show { kind != 0 } else { b.bit()? };
            let error = kind == 3 || (kind == 0 && show) || b.bit()?;
            (kind, show, showable, error)
        };
        let disable_cdf_update = b.bit()?;
        let screen_content = if s.screen_content_tools == 2 {
            b.bit()?
        } else {
            s.screen_content_tools == 1
        };
        let intra = frame_type == 0 || frame_type == 2;
        let integer_mv = if screen_content {
            if s.integer_mv == 2 {
                b.bit()?
            } else {
                s.integer_mv == 1
            }
        } else {
            false
        };
        let integer_mv = integer_mv || intra;
        let frame_id = s
            .frame_id_bits
            .map(|(_, total)| b.read(total))
            .transpose()?;
        let override_size = frame_type == 3 || (!s.reduced_header && b.bit()?);
        let order_hint = b.read(s.order_hint_bits)?;
        let primary_reference = if intra || error_resilient {
            7
        } else {
            b.read(3)? as usize
        };
        if let Some(model) = &s.decoder_model {
            if b.bit()? {
                for op in &s.operating_points {
                    if op.delays.is_some()
                        && (op.idc == 0
                            || (op.idc & (1 << temporal_id) != 0
                                && op.idc & (1 << (spatial_id + 8)) != 0))
                    {
                        b.read(model.removal_bits)?;
                    }
                }
            }
        }
        let refresh_flags = if frame_type == 3 || (frame_type == 0 && show) {
            255
        } else {
            b.read(8)? as u8
        };
        if (!intra || refresh_flags != 255) && error_resilient && s.order_hint_bits > 0 {
            for (i, reference) in adjusted_refs.iter_mut().enumerate() {
                let hint = b.read(s.order_hint_bits)?;
                if reference_order_hints[i] != hint {
                    *reference = None;
                    invalidated_references |= 1 << i;
                }
            }
        }
        let refs = &adjusted_refs;
        let mut references = [0usize; 7];
        let mut found_ref = None;
        if !intra {
            let short = s.order_hint_bits > 0 && b.bit()?;
            if short {
                references = short_references(
                    order_hint,
                    s.order_hint_bits,
                    b.read(3)? as usize,
                    b.read(3)? as usize,
                    reference_order_hints,
                )?;
            }
            for index in &mut references {
                if !short {
                    *index = b.read(3)? as usize;
                }
                if refs[*index].is_none() {
                    return Err(invalid("AV1 inter frame references missing picture"));
                }
                if let Some((delta_bits, id_bits)) = s.frame_id_bits {
                    let delta = b.read(delta_bits)? + 1;
                    let expected =
                        (frame_id.unwrap() + (1u32 << id_bits) - delta) % (1u32 << id_bits);
                    if refs[*index].and_then(|reference| reference.frame_id) != Some(expected) {
                        return Err(invalid("AV1 inter reference frame ID mismatch"));
                    }
                }
            }
            if override_size && !error_resilient {
                for index in references {
                    if b.bit()? {
                        found_ref = refs[index];
                        break;
                    }
                }
            }
        }
        let mut size = if let Some(reference) = found_ref {
            [reference.upscaled_width, reference.size[1]]
        } else if override_size {
            [
                b.read(s.dimension_bits[0])? + 1,
                b.read(s.dimension_bits[1])? + 1,
            ]
        } else {
            s.max_size
        };
        if size[0] > s.max_size[0] || size[1] > s.max_size[1] {
            return Err(invalid("AV1 frame exceeds sequence dimensions"));
        }
        let upscaled_width = size[0];
        let superres_denom = if s.superres && b.bit()? {
            b.read(3)? as u8 + 9
        } else {
            8
        };
        size[0] = (size[0] * 8 + u32::from(superres_denom) / 2) / u32::from(superres_denom);
        let render_size = if let Some(reference) = found_ref {
            reference.render_size
        } else if b.bit()? {
            [b.read(16)? + 1, b.read(16)? + 1]
        } else {
            [upscaled_width, size[1]]
        };
        let intrabc = intra && screen_content && size[0] == upscaled_width && b.bit()?;
        let mut high_precision_mv = false;
        let mut interpolation_filter = 0;
        let mut motion_mode_switchable = false;
        let mut reference_mvs = false;
        if !intra {
            high_precision_mv = !integer_mv && b.bit()?;
            interpolation_filter = if b.bit()? { 4 } else { b.read(2)? as usize };
            motion_mode_switchable = b.bit()?;
            reference_mvs = !error_resilient && s.reference_mvs && b.bit()?;
        }
        let primary = if primary_reference == 7 {
            None
        } else {
            refs[references[primary_reference]]
        };
        let disable_frame_end_update = s.reduced_header || disable_cdf_update || b.bit()?;
        let tiles = Layout::parse(b, size, s.superblock128)?;
        let base = b.read(8)? as u8;
        let mut delta = [0; 5];
        delta[0] = delta_q(b)?;
        if !s.color.monochrome {
            let diff_uv = s.color.separate_uv_delta_q && b.bit()?;
            delta[1] = delta_q(b)?;
            delta[2] = delta_q(b)?;
            if diff_uv {
                delta[3] = delta_q(b)?;
                delta[4] = delta_q(b)?;
            } else {
                delta[3] = delta[1];
                delta[4] = delta[2];
            }
        }
        let matrix = if b.bit()? {
            let y = b.read(4)? as u8;
            let u = b.read(4)? as u8;
            let v = if s.color.separate_uv_delta_q {
                b.read(4)? as u8
            } else {
                u
            };
            Some([y, u, v])
        } else {
            None
        };
        let mut segments = [[None; 8]; 8];
        let segmentation_enabled = b.bit()?;
        let mut segmentation_update_map = false;
        let mut segmentation_temporal_update = false;
        let mut segmentation_update_data = false;
        if segmentation_enabled {
            if primary_reference == 7 {
                segmentation_update_map = true;
                segmentation_update_data = true;
            } else {
                segmentation_update_map = b.bit()?;
                segmentation_temporal_update = segmentation_update_map && b.bit()?;
                segmentation_update_data = b.bit()?;
                segments = primary
                    .ok_or_else(|| invalid("missing AV1 primary segmentation reference"))?
                    .segments;
            }
            if segmentation_update_data {
                segments = [[None; 8]; 8];
                for segment in &mut segments {
                    for (j, feature) in segment.iter_mut().enumerate() {
                        if b.bit()? {
                            let n = [8, 6, 6, 6, 6, 3, 0, 0][j];
                            let limit = [255, 63, 63, 63, 63, 7, 0, 0][j];
                            let value = if j < 5 {
                                signed(b, n + 1)?
                            } else {
                                b.read(n)? as i32
                            };
                            *feature = Some(value.clamp(-limit, limit));
                        }
                    }
                }
            }
        }
        let delta_resolution = if base != 0 && b.bit()? {
            Some(b.read(2)? as u8)
        } else {
            None
        };
        let lf_delta_resolution = if delta_resolution.is_some() && !intrabc && b.bit()? {
            Some((b.read(2)? as u8, b.bit()?))
        } else {
            None
        };
        let quant = Quantization {
            base,
            delta,
            matrix,
            delta_resolution,
        };
        let lossless = segments.map(|seg| {
            (i32::from(base) + seg[0].unwrap_or(0)).clamp(0, 255) == 0 && delta == [0; 5]
        });
        let coded_lossless = lossless.iter().all(|v| *v);
        let all_lossless = coded_lossless && size[0] == upscaled_width;
        let mut filter = LoopFilter {
            levels: [0; 4],
            sharpness: 0,
            deltas_enabled: false,
            reference_deltas: [1, 0, 0, 0, -1, 0, -1, -1],
            mode_deltas: [0; 2],
            delta_resolution: lf_delta_resolution,
        };
        if let Some(primary) = primary {
            filter.reference_deltas = primary.filter.reference_deltas;
            filter.mode_deltas = primary.filter.mode_deltas;
        }
        if !coded_lossless && !intrabc {
            filter.levels[0] = b.read(6)? as u8;
            filter.levels[1] = b.read(6)? as u8;
            if !s.color.monochrome && filter.levels[..2] != [0, 0] {
                filter.levels[2] = b.read(6)? as u8;
                filter.levels[3] = b.read(6)? as u8;
            }
            filter.sharpness = b.read(3)? as u8;
            filter.deltas_enabled = b.bit()?;
            if filter.deltas_enabled && b.bit()? {
                for d in &mut filter.reference_deltas {
                    if b.bit()? {
                        *d = signed(b, 7)?;
                    }
                }
                for d in &mut filter.mode_deltas {
                    if b.bit()? {
                        *d = signed(b, 7)?;
                    }
                }
            }
        }
        let mut cdef = Cdef {
            damping: 3,
            bits: 0,
            strengths: [[0; 4]; 8],
        };
        if !coded_lossless && !intrabc && s.cdef {
            cdef.damping = b.read(2)? as u8 + 3;
            cdef.bits = b.read(2)? as u8;
            for strength in cdef.strengths.iter_mut().take(1 << cdef.bits) {
                for plane in 0..if s.color.monochrome { 1 } else { 2 } {
                    strength[plane * 2] = b.read(4)? as u8;
                    let secondary = b.read(2)? as u8;
                    strength[plane * 2 + 1] = if secondary == 3 { 4 } else { secondary };
                }
            }
        }
        let mut restoration_types = [0; 3];
        let mut restoration_sizes = [0; 3];
        if !all_lossless && !intrabc && s.restoration {
            for t in restoration_types
                .iter_mut()
                .take(if s.color.monochrome { 1 } else { 3 })
            {
                // Bitstream codes map NONE/SWITCHABLE/WIENER/SGRPROJ to
                // the semantic NONE/WIENER/SGRPROJ/SWITCHABLE enum.
                *t = [0, 3, 1, 2][b.read(2)? as usize];
            }
            if restoration_types != [0; 3] {
                let mut shift = b.read(1)? + u32::from(s.superblock128);
                if !s.superblock128 && shift != 0 {
                    shift += b.read(1)?;
                }
                restoration_sizes[0] = 256 >> (2 - shift);
                let uv_shift =
                    if s.color.subsampling == [true, true] && restoration_types[1..] != [0, 0] {
                        b.read(1)?
                    } else {
                        0
                    };
                restoration_sizes[1] = restoration_sizes[0] >> uv_shift;
                restoration_sizes[2] = restoration_sizes[1];
            }
        }
        let tx_mode = if coded_lossless {
            0
        } else if b.bit()? {
            2
        } else {
            1
        };
        let reference_select = !intra && b.bit()?;
        let mut skip_mode = None;
        if reference_select && s.order_hint_bits > 0 {
            let hint = |i: usize| refs[references[i]].unwrap().order_hint;
            let distance = |a: u32, b: u32| {
                let d = a.wrapping_sub(b) as i32;
                let m = 1 << (s.order_hint_bits - 1);
                (d & (m - 1)) - (d & m)
            };
            let forward = (0..7)
                .filter(|i| distance(hint(*i), order_hint) < 0)
                .min_by_key(|i| (-distance(hint(*i), order_hint), *i));
            if let Some(f) = forward {
                let backward = (0..7)
                    .filter(|i| distance(hint(*i), order_hint) > 0)
                    .min_by_key(|i| distance(hint(*i), order_hint));
                let other = backward.or_else(|| {
                    (0..7)
                        .filter(|i| distance(hint(*i), hint(f)) < 0)
                        .min_by_key(|i| (-distance(hint(*i), hint(f)), *i))
                });
                if let Some(o) = other {
                    if b.bit()? {
                        skip_mode = Some([1 + f.min(o), 1 + f.max(o)]);
                    }
                }
            }
        }
        let warped_motion = !intra && !error_resilient && s.warped_motion && b.bit()?;
        let reduced_tx_set = b.bit()?;
        let mut global_types = [0; 7];
        let mut global_params = [IDENTITY_GLOBAL; 7];
        if !intra {
            let previous = primary.map_or([IDENTITY_GLOBAL; 7], |h| h.global_params);
            for reference in 0..7 {
                let kind = if !b.bit()? {
                    0
                } else if b.bit()? {
                    2
                } else if b.bit()? {
                    1
                } else {
                    3
                };
                global_types[reference] = kind;
                for index in [2, 3, 4, 5, 0, 1].into_iter().filter(|&index| {
                    kind >= 1 && (index < 2 || kind >= 2 && (index < 4 || kind == 3))
                }) {
                    let (abs, precision) = if index >= 2 {
                        (12, 15)
                    } else if kind == 1 {
                        (
                            9 - u32::from(!high_precision_mv),
                            3 - u32::from(!high_precision_mv),
                        )
                    } else {
                        (12, 6)
                    };
                    let shift = 16 - precision;
                    let diagonal = index % 3 == 2;
                    let center = if diagonal { 65536 } else { 0 };
                    let sub = if diagonal { 1 << precision } else { 0 };
                    let r = (previous[reference][index] >> shift) - sub;
                    let maximum = 1i64 << abs;
                    global_params[reference][index] =
                        (global_parameter(b, -maximum, maximum + 1, r)? << shift) + center;
                }
                if kind == 2 {
                    global_params[reference][4] = -global_params[reference][3];
                    global_params[reference][5] = global_params[reference][2];
                }
            }
        }
        let grain = Grain::parse(
            b,
            s,
            frame_type,
            show || showable,
            references,
            refs.map(|h| h.map(|h| h.grain.as_ref())),
        )?;
        if separate {
            if !b.bit()? {
                return Err(invalid("missing AV1 frame header trailing one bit"));
            }
            while b.position() < data.len() * 8 {
                if b.bit()? {
                    return Err(invalid("nonzero AV1 frame header trailing padding"));
                }
            }
        } else {
            align(b)?;
        }
        Ok(Self {
            grain,
            frame_type,
            frame_id,
            show,
            showable,
            error_resilient,
            disable_cdf_update,
            disable_frame_end_update,
            screen_content,
            intrabc,
            order_hint,
            refresh_flags,
            invalidated_references,
            size_override: override_size,
            size,
            render_size,
            upscaled_width,
            superres_denom,
            tiles,
            quant,
            segments,
            segmentation_enabled,
            segmentation_update_map,
            segmentation_temporal_update,
            segmentation_update_data,
            lossless,
            filter,
            cdef,
            restoration_types,
            restoration_sizes,
            tx_mode,
            reduced_tx_set,
            header_bytes: b.position() / 8,
            primary_reference,
            references,
            integer_mv,
            high_precision_mv,
            interpolation_filter,
            motion_mode_switchable,
            reference_mvs,
            reference_select,
            skip_mode,
            warped_motion,
            global_types,
            global_params,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::av1::Obus;
    #[test]
    fn first_keyframe_header_matches_trace() {
        let obus = Obus::new(include_bytes!("../../tests/fixtures/av1/sequence.obu"))
            .collect::<Result<Vec<_>>>()
            .unwrap();
        let s = Sequence::parse(obus.iter().find(|o| o.kind == 1).unwrap().payload).unwrap();
        let o = obus.iter().find(|o| o.kind == 6).unwrap();
        let h = Header::parse_intra(&s, o.payload, o.temporal_id, o.spatial_id).unwrap();
        assert_eq!(h.size, [64, 64]);
        assert_eq!(h.tiles.count(), 1);
        assert_eq!(h.refresh_flags, 255);
        assert_eq!(h.quant.base, 86);
        assert!(
            !h.tiles.group(&o.payload[h.header_bytes..]).unwrap()[0]
                .1
                .is_empty()
        );
        for end in 0..h.header_bytes {
            assert!(Header::parse_intra(&s, &o.payload[..end], 0, 0).is_err());
        }
    }
}
