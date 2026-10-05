//! Spatial HEVC merge/AMVP candidates and quarter/eighth-sample interpolation.
use super::{
    hevc_inter_syntax::{Partition, Prediction},
    hevc_picture::Picture,
    hevc_slice::SliceHeader,
};
use crate::{Result, invalid};
use std::sync::Arc;
#[derive(Clone)]
pub struct Reference {
    pub long_term: bool,
    pub poc: i32,
    pub picture: Arc<Picture>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vector {
    pub reference: u8,
    pub mv: [i16; 2],
}
pub type Motion = [Option<Vector>; 2];
/// Preserve the referenced POC when publishing slice-local motion indices in a
/// picture-wide map used by deblocking and later temporal prediction.
pub(crate) fn remap_references(
    mut motion: Motion,
    source: &[Vec<i32>; 2],
    target: &[Vec<i32>; 2],
) -> Result<Motion> {
    for list in 0..2 {
        if let Some(vector) = &mut motion[list] {
            let poc = source[list]
                .get(vector.reference as usize)
                .ok_or_else(|| invalid("HEVC slice motion reference is out of range"))?;
            let index = target[list]
                .iter()
                .position(|v| v == poc)
                .ok_or_else(|| invalid("HEVC motion POC is absent from picture map"))?;
            vector.reference = u8::try_from(index)
                .map_err(|_| invalid("HEVC motion reference map exceeds index range"))?;
        }
    }
    Ok(motion)
}
pub struct Spatial<'a> {
    pub rect: [u32; 4],
    pub cu: [u32; 3],
    pub partition: Partition,
    pub part_index: usize,
    pub merge_log2: u8,
    pub ctu_log2: u8,
    pub poc: i32,
    pub lists: &'a [Vec<Reference>; 2],
}
impl Spatial<'_> {
    fn temporal(
        &self,
        header: &SliceHeader,
        list: usize,
        reference: u8,
    ) -> Result<Option<[i16; 2]>> {
        let collocated = self.lists[header.collocated_list]
            .get(header.collocated_ref as usize)
            .ok_or_else(|| invalid("HEVC collocated reference is missing"))?;
        let p = &collocated.picture;
        let [x, y, w, h] = self.rect;
        let low_delay = self.lists.iter().flatten().all(|r| r.poc <= self.poc);
        for (index, [xx, yy]) in [[x + w, y + h], [x + w / 2, y + h / 2]]
            .into_iter()
            .enumerate()
        {
            if index == 0
                && ((y >> self.ctu_log2) != (yy >> self.ctu_log2)
                    || xx >= p.dimensions[0]
                    || yy >= p.dimensions[1])
            {
                continue;
            }
            let Some(motion) = p
                .motion
                .get((yy / 16 * p.dimensions[0].div_ceil(16) + xx / 16) as usize)
            else {
                continue;
            };
            let source_list = if motion[0].is_none() {
                1
            } else if motion[1].is_none() {
                0
            } else if low_delay {
                list
            } else {
                1 - header.collocated_list
            };
            let Some(vector) = motion[source_list] else {
                continue;
            };
            let source_poc = *p.reference_pocs[source_list]
                .get(vector.reference as usize)
                .ok_or_else(|| invalid("HEVC collocated motion has invalid reference"))?;
            let target = self.lists[list]
                .get(reference as usize)
                .ok_or_else(|| invalid("HEVC temporal target reference is missing"))?;
            let source_long = *p.reference_long_term[source_list]
                .get(vector.reference as usize)
                .ok_or_else(|| invalid("HEVC collocated reference classification is missing"))?;
            if let Some(mv) = reference_predictor(vector.mv,
                collocated.poc.saturating_sub(source_poc),
                self.poc.saturating_sub(target.poc), source_long, target.long_term)? {
                return Ok(Some(mv));
            }
        }
        Ok(None)
    }
    pub fn resolve(
        &self,
        syntax: Prediction,
        header: &SliceHeader,
        at: impl Fn(i32, i32) -> Option<Motion>,
    ) -> Result<Motion> {
        let [x, y, w, h] = self.rect.map(|v| v as i32);
        let sample = |xx, yy| {
            if self.partition == Partition::Quarters
                && self.part_index == 1
                && xx >= self.cu[0] as i32
                && xx < (self.cu[0] + self.cu[2] / 2) as i32
                && yy >= (self.cu[1] + self.cu[2] / 2) as i32
                && yy < (self.cu[1] + self.cu[2]) as i32
            {
                return None;
            }
            at(xx, yy)
        };
        let points = [
            [x - 1, y + h - 1],
            [x + w - 1, y - 1],
            [x + w, y - 1],
            [x - 1, y + h],
            [x - 1, y - 1],
        ];
        match syntax {
            Prediction::Merge(index) => {
                if self.merge_log2 > 2 && self.cu[2] == 8 && self.partition != Partition::Full {
                    let shared = Spatial {
                        rect: [self.cu[0], self.cu[1], 8, 8],
                        partition: Partition::Full,
                        part_index: 0,
                        ..*self
                    };
                    return shared.resolve(syntax, header, at);
                }
                let mut raw = points.map(|[xx, yy]| {
                    if x >> self.merge_log2 == xx >> self.merge_log2
                        && y >> self.merge_log2 == yy >> self.merge_log2
                    {
                        None
                    } else {
                        sample(xx, yy)
                    }
                });
                if self.part_index == 1 {
                    if matches!(
                        self.partition,
                        Partition::Vertical | Partition::Left | Partition::Right
                    ) {
                        raw[0] = None;
                    }
                    if matches!(
                        self.partition,
                        Partition::Horizontal | Partition::Top | Partition::Bottom
                    ) {
                        raw[1] = None;
                    }
                }
                let mut candidates = Vec::with_capacity(5);
                if let Some(v) = raw[0] {
                    candidates.push(v);
                }
                if let Some(v) = raw[1] {
                    if raw[0] != Some(v) {
                        candidates.push(v);
                    }
                }
                if let Some(v) = raw[2] {
                    if raw[1] != Some(v) {
                        candidates.push(v);
                    }
                }
                if let Some(v) = raw[3] {
                    if raw[0] != Some(v) {
                        candidates.push(v);
                    }
                }
                if let Some(v) = raw[4] {
                    if candidates.len() < 4 && raw[0] != Some(v) && raw[1] != Some(v) {
                        candidates.push(v);
                    }
                }
                let max = header.max_merge_candidates as usize;
                if candidates.len() < max && header.temporal_mvp {
                    let mut temporal = [None; 2];
                    for list in 0..2 {
                        if header.references[list] > 0 {
                            temporal[list] = self
                                .temporal(header, list, 0)?
                                .map(|mv| Vector { reference: 0, mv });
                        }
                    }
                    if temporal.iter().any(Option::is_some) {
                        candidates.push(temporal);
                    }
                }
                candidates.truncate(max);
                if header.references[1] > 0 && candidates.len() < max {
                    let original = candidates.len();
                    for [a, b] in [
                        [0, 1],
                        [1, 0],
                        [0, 2],
                        [2, 0],
                        [1, 2],
                        [2, 1],
                        [0, 3],
                        [3, 0],
                        [1, 3],
                        [3, 1],
                        [2, 3],
                        [3, 2],
                    ]
                    .into_iter()
                    .take(original * (original.saturating_sub(1)))
                    {
                        if let (Some(l), Some(r)) = (candidates[a][0], candidates[b][1]) {
                            if self.lists[0][l.reference as usize].poc
                                != self.lists[1][r.reference as usize].poc
                                || l.mv != r.mv
                            {
                                candidates.push([Some(l), Some(r)]);
                            }
                        }
                        if candidates.len() == max {
                            break;
                        }
                    }
                }
                let refs = if header.references[1] > 0 {
                    header.references[0].min(header.references[1])
                } else {
                    header.references[0]
                };
                let mut zero = 0;
                while candidates.len() < max {
                    let v = Vector {
                        reference: if zero < refs { zero } else { 0 },
                        mv: [0; 2],
                    };
                    candidates.push([Some(v), (header.references[1] > 0).then_some(v)]);
                    zero += 1;
                }
                let mut result = *candidates
                    .get(index as usize)
                    .ok_or_else(|| invalid("HEVC merge index out of range"))?;
                if w + h == 12 && result.iter().all(Option::is_some) {
                    result[1] = None;
                }
                Ok(result)
            }
            Prediction::Explicit {
                references,
                differences,
                predictors,
            } => {
                let a = [sample(x - 1, y + h), sample(x - 1, y + h - 1)];
                let b = [
                    sample(x + w, y - 1),
                    sample(x + w - 1, y - 1),
                    sample(x - 1, y - 1),
                ];
                let mut result = [None; 2];
                for list in 0..2 {
                    let Some(reference) = references[list] else {
                        continue;
                    };
                    let target = &self.lists[list][reference as usize];
                    let find =
                        |neighbours: &[Option<Motion>], scaled: bool| -> Result<Option<[i16; 2]>> {
                            for motion in neighbours.iter().flatten() {
                                for l in [list, 1 - list] {
                                    if let Some(v) = motion[l] {
                                        let source = &self.lists[l][v.reference as usize];
                                        if source.long_term != target.long_term { continue; }
                                        if source.poc == target.poc {
                                            return Ok(Some(v.mv));
                                        }
                                        if scaled {
                                            return reference_predictor(v.mv,
                                                self.poc.saturating_sub(source.poc),
                                                self.poc.saturating_sub(target.poc),
                                                source.long_term, target.long_term);
                                        }
                                    }
                                }
                            }
                            Ok(None)
                        };
                    let left_exists = a.iter().any(Option::is_some);
                    let mut left = find(&a, false)?;
                    if left.is_none() {
                        left = find(&a, true)?;
                    }
                    let mut top = find(&b, false)?;
                    if !left_exists {
                        if top.is_some() {
                            left = top;
                        }
                        top = find(&b, true)?;
                    }
                    let mut choices = Vec::with_capacity(2);
                    if let Some(v) = left {
                        choices.push(v);
                    }
                    if let Some(v) = top {
                        if choices.first() != Some(&v) {
                            choices.push(v);
                        }
                    }
                    if choices.len() < 2 && header.temporal_mvp {
                        if let Some(mv) = self.temporal(header, list, reference)? {
                            choices.push(mv);
                        }
                    }
                    choices.resize(2, [0; 2]);
                    let predicted = choices[predictors[list]];
                    result[list] = Some(Vector {
                        reference,
                        mv: std::array::from_fn(|i| {
                            predicted[i].wrapping_add(differences[list][i])
                        }),
                    });
                }
                Ok(result)
            }
        }
    }
}
fn reference_predictor(mv: [i16; 2], source: i32, target: i32,
    source_long: bool, target_long: bool) -> Result<Option<[i16; 2]>> {
    if source_long != target_long { return Ok(None); }
    if target_long { return Ok(Some(mv)); }
    Ok(Some(scale(mv, source, target)?))
}
pub fn scale(mv: [i16; 2], source: i32, target: i32) -> Result<[i16; 2]> {
    if source == target {
        return Ok(mv);
    }
    let td = source.clamp(-128, 127);
    let tb = target.clamp(-128, 127);
    if td == 0 {
        return Err(invalid("HEVC motion scaling has zero POC distance"));
    }
    let tx = (16384 + (td.abs() >> 1)) / td;
    let factor = ((tb * tx + 32) >> 6).clamp(-4096, 4095);
    Ok(mv.map(|v| {
        let product = factor * i32::from(v);
        (product.signum() * ((product.abs() + 127) >> 8)).clamp(-32768, 32767) as i16
    }))
}
const LUMA: [[i32; 8]; 4] = [
    [0, 0, 0, 64, 0, 0, 0, 0],
    [-1, 4, -10, 58, 17, -5, 1, 0],
    [-1, 4, -11, 40, 40, -11, 4, -1],
    [0, 1, -5, 17, 58, -10, 4, -1],
];
const CHROMA: [[i32; 4]; 8] = [
    [0, 64, 0, 0],
    [-2, 58, 10, -2],
    [-4, 54, 16, -2],
    [-6, 46, 28, -4],
    [-4, 36, 36, -4],
    [-4, 28, 46, -6],
    [-2, 16, 54, -4],
    [-2, 10, 58, -2],
];
/// Separable interpolation retains the normative intermediate rounding.
fn interpolate_block(
    picture: &Picture,
    c: usize,
    rect: [u32; 4],
    mv: [i16; 2],
    scratch: &mut Vec<i32>,
    output: &mut [i32],
) {
    let [x, y, w, h] = rect.map(|v| v as usize);
    let chroma = usize::from(c != 0);
    let bits = 2 + chroma;
    let width = (picture.dimensions[0] >> chroma) as i32;
    let height = (picture.dimensions[1] >> chroma) as i32;
    let samples = picture.planes[c].samples();
    let depth = picture.depth[chroma];
    let x = x as i32 + (i32::from(mv[0]) >> bits);
    let y = y as i32 + (i32::from(mv[1]) >> bits);
    let fx = (i32::from(mv[0]) & ((1 << bits) - 1)) as usize;
    let fy = (i32::from(mv[1]) & ((1 << bits) - 1)) as usize;
    let filters = |f: usize| -> &[i32] { if c == 0 { &LUMA[f] } else { &CHROMA[f] } };
    let start = if c == 0 { -3 } else { -1 };
    let taps = if c == 0 { 8 } else { 4 };
    let extra = if fy == 0 { 0 } else { taps - 1 };
    if fx == 0 && fy == 0 {
        for row in 0..h {
            let yy = (y + row as i32).clamp(0, height - 1) as usize;
            let source = &samples[yy * width as usize..(yy + 1) * width as usize];
            let out_row = &mut output[row * w..(row + 1) * w];
            if x >= 0 && x as usize + w <= width as usize {
                for (out, &value) in out_row.iter_mut().zip(&source[x as usize..x as usize + w]) {
                    *out = i32::from(value) << (14 - depth);
                }
            } else {
                for (col, out) in out_row.iter_mut().enumerate() {
                    *out = i32::from(source[(x + col as i32).clamp(0, width - 1) as usize])
                        << (14 - depth);
                }
            }
        }
        return;
    }
    let horizontal_len = w * (h + extra);
    scratch.resize(horizontal_len, 0);
    let horizontal = &mut scratch[..horizontal_len];
    horizontal.fill(0);
    for row in 0..h + extra {
        let yy = (y + row as i32 + if fy == 0 { 0 } else { start }).clamp(0, height - 1);
        let source = &samples[yy as usize * width as usize..(yy as usize + 1) * width as usize];
        let target = &mut horizontal[row * w..(row + 1) * w];
        if fx == 0 {
            if x >= 0 && x as usize + w <= width as usize {
                for (out, &value) in target.iter_mut().zip(&source[x as usize..x as usize + w]) {
                    *out = i32::from(value) << (14 - depth);
                }
            } else {
                for (col, out) in target.iter_mut().enumerate() {
                    *out = i32::from(source[(x + col as i32).clamp(0, width - 1) as usize])
                        << (14 - depth);
                }
            }
        } else {
            for (k, &weight) in filters(fx).iter().enumerate() {
                if weight == 0 {
                    continue;
                }
                let left = x + start + k as i32;
                if left >= 0 && left as usize + w <= width as usize {
                    for (out, &value) in target
                        .iter_mut()
                        .zip(&source[left as usize..left as usize + w])
                    {
                        *out += weight * i32::from(value);
                    }
                } else {
                    for (col, out) in target.iter_mut().enumerate() {
                        *out += weight
                            * i32::from(source[(left + col as i32).clamp(0, width - 1) as usize]);
                    }
                }
            }
            for out in target {
                *out >>= depth - 8;
            }
        }
    }
    if fy == 0 {
        output[..w * h].copy_from_slice(horizontal);
        return;
    }
    for row in 0..h {
        let target = &mut output[row * w..(row + 1) * w];
        target.fill(0);
        for (k, &weight) in filters(fy).iter().enumerate() {
            if weight == 0 {
                continue;
            }
            for (out, &value) in target
                .iter_mut()
                .zip(&horizontal[(row + k) * w..(row + k + 1) * w])
            {
                *out += weight * value;
            }
        }
        for out in target {
            *out >>= 6;
        }
    }
}
pub fn predict(
    lists: &[Vec<Reference>; 2],
    motion: Motion,
    rect: [u32; 4],
    component: usize,
    depth: u8,
    weights: Option<&super::hevc_slice::Weights>,
    scratch: &mut Vec<i32>,
) -> Result<Vec<u16>> {
    let shift = usize::from(component != 0);
    let [x, y, w, h] = rect.map(|v| v >> shift);
    let mut inputs = [None; 2];
    let mut count = 0;
    for list in 0..2 {
        if let Some(v) = motion[list] {
            let weight = weights
                .map(|w| {
                    w.lists[list]
                        .get(v.reference as usize)
                        .ok_or_else(|| invalid("HEVC weight index out of range"))
                })
                .transpose()?;
            inputs[count] = Some((
                lists[list]
                    .get(v.reference as usize)
                    .ok_or_else(|| invalid("HEVC reference index out of range"))?,
                v.mv,
                weight.map_or(1, |w| i32::from(w.values[component])),
                weight.map_or(0, |w| {
                    let offset_shift = if weights.is_some_and(|w| w.high_precision_offsets) {
                        0
                    } else {
                        depth - 8
                    };
                    i32::from(w.offsets[component]) << offset_shift
                }),
            ));
            count += 1;
        }
    }
    if count == 0 {
        return Err(invalid("HEVC prediction has no reference"));
    }
    let first_input = inputs[0].unwrap();
    let second_input = inputs[1];
    let mask = (1 << (2 + shift)) - 1;
    if count == 1 && weights.is_none() && first_input.1.iter().all(|v| i32::from(*v) & mask == 0) {
        let picture = &first_input.0.picture;
        let width = (picture.dimensions[0] >> shift) as usize;
        let height = (picture.dimensions[1] >> shift) as i32;
        let xx = x as i32 + (i32::from(first_input.1[0]) >> (2 + shift));
        let yy = y as i32 + (i32::from(first_input.1[1]) >> (2 + shift));
        let mut output = Vec::with_capacity(w as usize * h as usize);
        for row in 0..h {
            let row = (yy + row as i32).clamp(0, height - 1) as usize;
            let source = &picture.planes[component].samples()[row * width..(row + 1) * width];
            if xx >= 0 && xx as usize + w as usize <= width {
                output.extend_from_slice(&source[xx as usize..xx as usize + w as usize]);
            } else {
                output.extend(
                    (0..w).map(|col| source[(xx + col as i32).clamp(0, width as i32 - 1) as usize]),
                );
            }
        }
        return Ok(output);
    }
    let precision = 14 - depth + weights.map_or(0, |w| w.denominators[shift]);
    let max = (1 << depth) - 1;
    let block_size = w as usize * h as usize;
    let mut output = vec![0u16; block_size];
    if count == 1 {
        let mut block_output = vec![0i32; block_size];
        interpolate_block(
            &first_input.0.picture,
            component,
            [x, y, w, h],
            first_input.1,
            scratch,
            &mut block_output,
        );
        for (out, &value) in output.iter_mut().zip(block_output.iter()) {
            *out = (((value * first_input.2 + (1 << (precision - 1))) >> precision) + first_input.3)
                .clamp(0, max) as u16;
        }
    } else {
        let second_input = second_input.unwrap();
        let mut first_result = vec![0i32; block_size];
        interpolate_block(
            &first_input.0.picture,
            component,
            [x, y, w, h],
            first_input.1,
            scratch,
            &mut first_result,
        );
        let mut second_result = vec![0i32; block_size];
        interpolate_block(
            &second_input.0.picture,
            component,
            [x, y, w, h],
            second_input.1,
            scratch,
            &mut second_result,
        );
        let offset = (first_input.3 + second_input.3 + 1) << precision;
        for ((out, &a), &b) in output
            .iter_mut()
            .zip(first_result.iter())
            .zip(second_result.iter())
        {
            *out = ((a * first_input.2 + b * second_input.2 + offset) >> (precision + 1))
                .clamp(0, max) as u16;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::hevc_plane::Plane;

    #[test]
    fn reference_classification_controls_temporal_scaling_and_availability() {
        let mv = [64, -32];
        assert_eq!(reference_predictor(mv, 4, 2, false, false).unwrap(), Some([32, -16]));
        assert_eq!(reference_predictor(mv, 4, 2, true, true).unwrap(), Some(mv));
        assert_eq!(reference_predictor(mv, 0, 100, true, true).unwrap(), Some(mv));
        assert_eq!(reference_predictor(mv, 4, 2, true, false).unwrap(), None);
        assert_eq!(reference_predictor(mv, 4, 2, false, true).unwrap(), None);
        assert!(reference_predictor(mv, 0, 2, false, false).is_err());
    }

    #[test]
    fn temporal_prediction_tries_center_after_a_reference_class_mismatch() {
        use crate::codec::{config::{HevcConfig, NalUnits}, hevc_decoder::HevcDecoder, hevc_nal::NalHeader};
        let data = include_bytes!("../../tests/fixtures/playback-errors/shared-hevc-main.mp4");
        let mut input = crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default()).unwrap();
        let configuration = &input.tracks()[0].configuration;
        let length = HevcConfig::parse(configuration).unwrap().length_size;
        let decoder = HevcDecoder::from_configuration(configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let nal = NalUnits::new(&packet, length).unwrap().map(|n| n.unwrap())
            .find(|n| NalHeader::parse(n).unwrap().is_vcl()).unwrap();
        let (sps, pps) = decoder.parameters();
        let header = SliceHeader::parse(nal, sps, pps, 16 << 20).unwrap();
        let mut motion = vec![[None, None]; 8];
        motion[5][0] = Some(Vector { reference: 0, mv: [12, 8] });
        motion[0][0] = Some(Vector { reference: 1, mv: [64, -32] });
        let picture = Arc::new(Picture {
            dimensions: [64, 32], crop: [0; 4], depth: [8; 2],
            planes: [Plane::new(64, 32, 8, 8192).unwrap(),
                Plane::new(32, 16, 8, 2048).unwrap(), Plane::new(32, 16, 8, 2048).unwrap()],
            sao: Vec::new(), motion,
            reference_pocs: [vec![0, 1], Vec::new()],
            reference_long_term: [vec![false, true], Vec::new()],
        });
        let lists = [vec![
            Reference { poc: 8, long_term: false, picture: Arc::clone(&picture) },
            Reference { poc: 1, long_term: true, picture },
        ], Vec::new()];
        let spatial = Spatial { rect: [0, 0, 16, 16], cu: [0, 0, 4],
            partition: Partition::Full, part_index: 0, merge_log2: 2, ctu_log2: 6,
            poc: 12, lists: &lists };
        assert_eq!(spatial.temporal(&header, 0, 1).unwrap(), Some([64, -32]));
    }

    #[test]
    fn separable_prediction_matches_scalar_at_all_phases_and_borders() {
        for depth in [8, 10, 12] {
            let mut picture = Picture {
                dimensions: [32, 32],
                crop: [0; 4],
                depth: [depth; 2],
                planes: [
                    Plane::new(32, 32, depth, 3072).unwrap(),
                    Plane::new(16, 16, depth, 768).unwrap(),
                    Plane::new(16, 16, depth, 768).unwrap(),
                ],
                sao: Vec::new(),
                motion: Vec::new(),
                reference_pocs: [Vec::new(), Vec::new()],
                reference_long_term: [Vec::new(), Vec::new()],
            };
            let mut state = 97u32;
            for plane in &mut picture.planes {
                for value in plane.samples_mut() {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    *value = (state >> 16) as u16 & ((1 << depth) - 1);
                }
            }
            for c in 0..3 {
                let bits = if c == 0 { 2 } else { 3 };
                let n = if c == 0 { 32 } else { 16 };
                let start = if c == 0 { -3 } else { -1 };
                let filter =
                    |phase: usize| -> &[i32] { if c == 0 { &LUMA[phase] } else { &CHROMA[phase] } };
                for fy in 0..1 << bits {
                    for fx in 0..1 << bits {
                        for (origin, displacement) in [(0, -3), (n - 8, 3)] {
                            let mv = [
                                (displacement << bits) + fx as i16,
                                (displacement << bits) + fy as i16,
                            ];
                            let mut scratch = Vec::new();
                            let mut output = vec![0i32; 64];
                            interpolate_block(
                                &picture,
                                c,
                                [origin as u32, origin as u32, 8, 8],
                                mv,
                                &mut scratch,
                                &mut output,
                            );
                            for y in 0..8 {
                                for x in 0..8 {
                                    let sample = |dx: i32, dy: i32| {
                                        let xx = (origin + x + i32::from(displacement) + dx)
                                            .clamp(0, n - 1);
                                        let yy = (origin + y + i32::from(displacement) + dy)
                                            .clamp(0, n - 1);
                                        i32::from(
                                            picture.planes[c].samples()[(yy * n + xx) as usize],
                                        )
                                    };
                                    let expected = if fx == 0 && fy == 0 {
                                        sample(0, 0) << (14 - depth)
                                    } else if fy == 0 {
                                        filter(fx)
                                            .iter()
                                            .enumerate()
                                            .map(|(k, v)| v * sample(start + k as i32, 0))
                                            .sum::<i32>()
                                            >> (depth - 8)
                                    } else if fx == 0 {
                                        filter(fy)
                                            .iter()
                                            .enumerate()
                                            .map(|(k, v)| v * sample(0, start + k as i32))
                                            .sum::<i32>()
                                            >> (depth - 8)
                                    } else {
                                        filter(fy)
                                            .iter()
                                            .enumerate()
                                            .map(|(k, v)| {
                                                let horizontal = filter(fx)
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(j, u)| {
                                                        u * sample(
                                                            start + j as i32,
                                                            start + k as i32,
                                                        )
                                                    })
                                                    .sum::<i32>()
                                                    >> (depth - 8);
                                                v * horizontal
                                            })
                                            .sum::<i32>()
                                            >> 6
                                    };
                                    assert_eq!(
                                        output[(y * 8 + x) as usize],
                                        expected,
                                        "depth={depth} c={c} phase={fx},{fy}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod reference_remap_tests {
    #[test]
    fn reordered_and_overlapping_slice_lists_preserve_poc_and_vector() {
        use super::{Vector, remap_references};
        let source = [vec![12, 8], vec![20, 16]];
        let target = [vec![8, 4, 12], vec![16, 20, 24]];
        let motion = [
            Some(Vector {
                reference: 0,
                mv: [17, -3],
            }),
            Some(Vector {
                reference: 1,
                mv: [-7, 11],
            }),
        ];
        let remapped = remap_references(motion, &source, &target).unwrap();
        assert_eq!(
            remapped[0].unwrap(),
            Vector {
                reference: 2,
                mv: [17, -3]
            }
        );
        assert_eq!(
            remapped[1].unwrap(),
            Vector {
                reference: 0,
                mv: [-7, 11]
            }
        );
        assert!(remap_references(motion, &[vec![], vec![]], &target).is_err());
        assert!(remap_references(motion, &source, &[vec![8], vec![16]]).is_err());
    }
}
