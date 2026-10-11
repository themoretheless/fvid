//! Annex F dependency/layer-set prefixes and strict single-layer VPS extension.
//! Ordinary Annex A may ignore errors in the deferred single-layer tail.
use super::{bits::BitReader, hevc_profile::ProfileTierLevel};
use crate::{Result, invalid};

/// Annex F extension syntax through direct_dependency_flag, before additional
/// sets/OLS/DPB. Masks use NAL layer IDs, not VPS indices. This is deliberately
/// a prefix reader, not admission of a complete multilayer picture stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerDependencyPrefix {
    pub layer_ids: Vec<u8>,
    pub scalability_ids: Vec<[u8; 16]>,
    pub view_ids: Vec<u16>,
    pub direct: Vec<u64>,
    pub transitive: Vec<u64>,
    pub tree_partitions: Vec<Vec<u8>>,
    pub end_bit: usize,
}
/// Annex F layer sets and temporal inter-layer admission metadata. Layer lists
/// preserve normative tree-partition order; they are not sorted bit masks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerSetExtension {
    pub layer_sets: Vec<Vec<u8>>,
    pub sublayers_minus1: Vec<u8>,
    pub set_sublayers_minus1: Vec<u8>,
    /// Indexed [reference VPS index][dependent VPS index], inferred value 7.
    pub max_tid_ref_plus1: Vec<Vec<u8>>,
    pub default_ref_layers_active: bool,
    pub end_bit: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLayerSet {
    pub layer_set_index: usize,
    pub output: u64,
    pub necessary: u64,
    /// Aligned with the associated layer list; non-necessary layers have None.
    pub profile_indices: Vec<Option<u8>>,
    pub alternative_output: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLayerSets {
    pub profile_count: u8,
    /// Coded extra PTLs and their VPS indices; existing base PTLs are not cloned.
    pub extra_profiles: Vec<(u8, ProfileTierLevel)>,
    pub default_output_idc: u8,
    pub sets: Vec<OutputLayerSet>,
    pub end_bit: usize,
}
impl LayerSetExtension {
    /// Continue through PTL and OLS syntax, stopping before representation formats.
    pub fn output_layers(
        &self,
        bytes: &[u8],
        graph: &LayerDependencyPrefix,
        layers: &super::hevc_vps::LayerSets,
        sublayers: u8,
        budget: usize,
    ) -> Result<OutputLayerSets> {
        let n = graph.layer_ids.len();
        if n == 0
            || n > 63
            || graph.direct.len() != n
            || graph.transitive.len() != n
            || graph.layer_ids.iter().any(|&id| id > 63)
            || graph.layer_ids.windows(2).any(|w| w[0] >= w[1])
            || self.layer_sets.is_empty()
            || self.layer_sets.len() > 2047
            || self.layer_sets[0] != [0]
            || layers.sets.is_empty()
            || layers.sets.len() > self.layer_sets.len()
            || sublayers > 6
        {
            return Err(invalid("invalid HEVC output layer metadata geometry"));
        }
        for set in &self.layer_sets {
            let mut mask = 0u64;
            if set.is_empty() || set.len() > 63 {
                return Err(invalid("invalid HEVC output layer list"));
            }
            for &id in set {
                if id > 63
                    || graph.layer_ids.binary_search(&id).is_err()
                    || mask & (1u64 << id) != 0
                {
                    return Err(invalid("invalid HEVC output layer list"));
                }
                mask |= 1u64 << id;
            }
        }
        let mut b = BitReader::new(bytes);
        b.skip(self.end_bit)?;
        let last_profile = bounded(&mut b, 63)? as u8;
        if layers.max_layers_minus1 > 0 && last_profile == 0 {
            return Err(invalid("multilayer HEVC requires at least two PTLs"));
        }
        // Bound all retained PTLs, including sublayer records, before reading them.
        if budget < 32768 {
            return Err(invalid("HEVC output metadata exceeds memory budget"));
        }
        let mut profiles = Vec::new();
        for i in if layers.base_internal { 2 } else { 1 }..=last_profile {
            let present = b.bit()?;
            profiles.push((i, ProfileTierLevel::read(&mut b, present, sublayers)?));
        }
        let (extra, idc) = if self.layer_sets.len() > 1 {
            (bounded(&mut b, 1023)? as usize, b.read(2)? as u8)
        } else {
            (0, 0)
        };
        let count = self.layer_sets.len() + extra;
        if 32768 + count * (std::mem::size_of::<OutputLayerSet>() + 128) > budget {
            return Err(invalid("HEVC output metadata exceeds memory budget"));
        }
        let mut sets = Vec::with_capacity(count);
        sets.push(OutputLayerSet {
            layer_set_index: 0,
            output: 1,
            necessary: 1,
            profile_indices: vec![Some(u8::from(
                layers.base_internal && layers.max_layers_minus1 > 0,
            ))],
            alternative_output: false,
        });
        let mut covered = 1u64;
        for i in 1..count {
            let index = if i < self.layer_sets.len() {
                i
            } else if self.layer_sets.len() > 2 {
                let width = (usize::BITS - (self.layer_sets.len() - 2).leading_zeros()) as u8;
                let value = b.read(width)? as usize + 1;
                if value >= self.layer_sets.len() {
                    return Err(invalid("HEVC OLS layer set index out of range"));
                }
                value
            } else {
                1
            };
            let list = &self.layer_sets[index];
            let explicit = i >= layers.sets.len() || idc.min(2) == 2;
            let highest = *list.iter().max().unwrap();
            let mut output = 0u64;
            let mut last = 0;
            for &id in list {
                let flag = if explicit {
                    b.bit()?
                } else {
                    idc == 0 || id == highest
                };
                if flag {
                    output |= 1u64 << id;
                    last = id;
                }
            }
            if output == 0 {
                return Err(invalid("empty HEVC output layer set"));
            }
            let mut necessary = output;
            for (position, &id) in list.iter().enumerate() {
                if output & (1u64 << id) == 0 {
                    continue;
                }
                let references = graph.transitive[graph.layer_ids.binary_search(&id).unwrap()];
                for &reference in &list[..position] {
                    if references & (1u64 << reference) != 0 {
                        necessary |= 1u64 << reference;
                    }
                }
            }
            if index >= layers.sets.len()
                && necessary.count_ones() == 1
                && necessary & (1u64 << list[0]) == 0
            {
                return Err(invalid("HEVC additional OLS singleton is not first layer"));
            }
            let mut indices = vec![None; list.len()];
            let width = (u8::BITS - last_profile.leading_zeros()) as u8;
            for (j, &id) in list.iter().enumerate() {
                if necessary & (1u64 << id) == 0 {
                    continue;
                }
                let value = if last_profile > 0 {
                    b.read(width)? as u8
                } else {
                    0
                };
                if value > last_profile || (!layers.base_internal && last_profile > 0 && value == 0)
                {
                    return Err(invalid("HEVC OLS PTL index out of range"));
                }
                indices[j] = Some(value);
            }
            let alternative = if output.count_ones() == 1
                && graph.direct[graph.layer_ids.binary_search(&last).unwrap()] != 0
            {
                b.bit()?
            } else {
                false
            };
            covered |= necessary;
            sets.push(OutputLayerSet {
                layer_set_index: index,
                output,
                necessary,
                profile_indices: indices,
                alternative_output: alternative,
            });
        }
        for &id in graph
            .layer_ids
            .iter()
            .skip(usize::from(!layers.base_internal))
        {
            if covered & (1u64 << id) == 0 {
                return Err(invalid("HEVC layer is unnecessary in every OLS"));
            }
        }
        Ok(OutputLayerSets {
            profile_count: last_profile + 1,
            extra_profiles: profiles,
            default_output_idc: idc,
            sets,
            end_bit: b.position(),
        })
    }
}
impl LayerDependencyPrefix {
    /// Continue at this prefix's end through default_ref_layers_active_flag.
    /// Does not consume PTL/OLS/representation/DPB syntax which follows it.
    pub fn layer_sets(
        &self,
        bytes: &[u8],
        layers: &super::hevc_vps::LayerSets,
        sublayers: u8,
        budget: usize,
    ) -> Result<LayerSetExtension> {
        let n = self.layer_ids.len();
        if layers.sets.first() != Some(&1) {
            return Err(invalid("HEVC layer sets lack inferred base set"));
        }
        // Public metadata can be constructed by callers: validate indexing before use.
        if n == 0
            || n > 63
            || self.direct.len() != n
            || self.layer_ids[0] != 0
            || self.layer_ids.windows(2).any(|w| w[0] >= w[1])
            || self.layer_ids.iter().any(|&id| id > 63)
            || sublayers > 6
        {
            return Err(invalid("invalid HEVC dependency prefix geometry"));
        }
        let known = self.layer_ids.iter().fold(0u64, |m, &id| m | (1u64 << id));
        let mut seen = 0u64;
        for tree in &self.tree_partitions {
            if tree.is_empty() {
                return Err(invalid("empty HEVC dependency tree"));
            }
            for &id in tree {
                if id > 63 || known & (1u64 << id) == 0 || seen & (1u64 << id) != 0 {
                    return Err(invalid("invalid HEVC dependency tree layer"));
                }
                seen |= 1u64 << id;
            }
        }
        if seen != known {
            return Err(invalid("incomplete HEVC dependency trees"));
        }
        let mut b = BitReader::new(bytes);
        b.skip(self.end_bit)?;
        let count = if self.tree_partitions.len() > 1 {
            bounded(&mut b, 1023)? as usize
        } else {
            0
        };
        if !layers.base_available && count == 0 {
            return Err(invalid(
                "unavailable HEVC base requires additional layer sets",
            ));
        }
        let total = layers
            .sets
            .len()
            .checked_add(count)
            .ok_or_else(|| invalid("HEVC layer set count overflow"))?;
        if layers.sets.len() > 1024
            || total * (std::mem::size_of::<Vec<u8>>() + 64 + 1)
                + n * n
                + n * std::mem::size_of::<Vec<u8>>()
                + 4096
                > budget
        {
            return Err(invalid("HEVC layer set extension exceeds memory budget"));
        }
        let mut sets = Vec::with_capacity(total);
        for &mask in &layers.sets {
            if mask == 0 || mask & !known != 0 {
                return Err(invalid("HEVC layer set names absent layer"));
            }
            sets.push(
                self.layer_ids
                    .iter()
                    .copied()
                    .filter(|&id| mask & (1u64 << id) != 0)
                    .collect(),
            );
        }
        for _ in 0..count {
            let mut set = Vec::with_capacity(n);
            // Tree zero is intentionally excluded, per F-9.
            for tree in self.tree_partitions.iter().skip(1) {
                let width = (usize::BITS - tree.len().leading_zeros()) as u8;
                let highest = b.read(width)? as usize;
                if highest > tree.len() {
                    return Err(invalid("HEVC highest tree layer index out of range"));
                }
                set.extend_from_slice(&tree[..highest]);
            }
            if set.is_empty() {
                return Err(invalid("empty additional HEVC layer set"));
            }
            sets.push(set);
        }
        let present = b.bit()?;
        let mut temporal = vec![sublayers; n];
        if present {
            for value in &mut temporal {
                *value = b.read(3)? as u8;
                if *value > sublayers {
                    return Err(invalid("HEVC layer exceeds VPS temporal sublayers"));
                }
            }
        }
        let set_temporal = sets
            .iter()
            .map(|set| {
                set.iter()
                    .map(|id| temporal[self.layer_ids.binary_search(id).unwrap()])
                    .max()
                    .unwrap()
            })
            .collect();
        let mut max_tid = vec![vec![7u8; n]; n];
        if b.bit()? {
            for i in 0..n {
                for j in i + 1..n {
                    if self.direct[j] & (1u64 << self.layer_ids[i]) != 0 {
                        max_tid[i][j] = b.read(3)? as u8;
                    }
                }
            }
        }
        let active = b.bit()?;
        Ok(LayerSetExtension {
            layer_sets: sets,
            sublayers_minus1: temporal,
            set_sublayers_minus1: set_temporal,
            max_tid_ref_plus1: max_tid,
            default_ref_layers_active: active,
            end_bit: b.position(),
        })
    }
    /// Input is unescaped VPS RBSP; start follows vps_extension_flag.
    /// The caller retains the remaining syntax for additional layer sets/OLS/DPB.
    pub fn parse(
        bytes: &[u8],
        start: usize,
        layers: &super::hevc_vps::LayerSets,
        sublayers: u8,
        budget: usize,
    ) -> Result<Self> {
        let n = usize::from(layers.effective_max_layers());
        // Conservative bound including vector growth, scratch dimensions and
        // temporary PTL sublayers. n is bounded by the normative maximum 63.
        if 4096 + n * 128 + n * n > budget {
            return Err(invalid("HEVC dependency metadata exceeds memory budget"));
        }
        let mut b = BitReader::new(bytes);
        b.skip(start)?;
        while b.position() % 8 != 0 {
            if !b.bit()? {
                return Err(invalid("HEVC VPS extension alignment bit is not one"));
            }
        }
        if layers.max_layers_minus1 > 0 && layers.base_internal {
            ProfileTierLevel::read(&mut b, false, sublayers)?;
        }
        let splitting = b.bit()?;
        let mut types = Vec::new();
        for i in 0..16 {
            if b.bit()? {
                types.push(i);
            }
        }
        if splitting && types.is_empty() {
            return Err(invalid("HEVC VPS splitting has no scalability dimensions"));
        }
        let mut widths = Vec::new();
        let mut offset = 0;
        for _ in 0..types.len() - usize::from(splitting) {
            let w = b.read(3)? as u8 + 1;
            widths.push(w);
            offset += usize::from(w);
        }
        if !types.is_empty() {
            let previous = if splitting {
                offset
            } else {
                offset - usize::from(*widths.last().unwrap())
            };
            if previous >= 6 {
                return Err(invalid("HEVC VPS split dimensions exceed layer ID"));
            }
        }
        if splitting {
            widths.push((6 - offset) as u8);
        }
        let explicit = b.bit()?;
        let mut ids = vec![0u8];
        let mut dimensions = vec![[0u8; 16]; n];
        for i in 1..n {
            let id = if explicit { b.read(6)? as u8 } else { i as u8 };
            if id <= ids[i - 1] || id > layers.max_layer_id {
                return Err(invalid(
                    "HEVC VPS layer IDs are not increasing or exceed maximum",
                ));
            }
            ids.push(id);
            let mut shift = 0;
            for (&t, &w) in types.iter().zip(&widths) {
                dimensions[i][t] = if splitting {
                    ((u32::from(id) >> shift) & ((1u32 << w) - 1)) as u8
                } else {
                    b.read(w)? as u8
                };
                shift += w;
            }
        }
        let mut views = Vec::new();
        for d in &dimensions {
            if !views.contains(&d[1]) {
                views.push(d[1]);
            }
        }
        let view_width = b.read(4)? as u8;
        let mut view_ids = Vec::new();
        for _ in &views {
            view_ids.push(if view_width == 0 {
                0
            } else {
                b.read(view_width)? as u16
            });
        }
        let mut direct = vec![0u64; n];
        let mut transitive = vec![0u64; n];
        for i in 1..n {
            for j in 0..i {
                if b.bit()? {
                    direct[i] |= 1u64 << ids[j];
                    transitive[i] |= (1u64 << ids[j]) | transitive[j];
                }
            }
        }
        let mut claimed = 0u64;
        let mut tree_partitions = Vec::new();
        for i in 0..n {
            if direct[i] != 0 {
                continue;
            }
            let mut tree = vec![ids[i]];
            for j in i + 1..n {
                let bit = 1u64 << ids[j];
                if transitive[j] & (1u64 << ids[i]) != 0 && claimed & bit == 0 {
                    tree.push(ids[j]);
                    claimed |= bit;
                }
            }
            tree_partitions.push(tree);
        }
        Ok(Self {
            layer_ids: ids,
            scalability_ids: dimensions,
            view_ids,
            direct,
            transitive,
            tree_partitions,
            end_bit: b.position(),
        })
    }
}

fn bounded(b: &mut BitReader<'_>, maximum: u32) -> Result<u32> {
    let value = b.unsigned_golomb()?;
    if value > maximum {
        return Err(invalid("HEVC VPS extension value out of range"));
    }
    Ok(value)
}

#[cfg(test)]
fn poc_lsb_aligned(bytes: &[u8], start: usize, sublayers: u8) -> Result<bool> {
    poc_lsb_aligned_with_hrd(bytes, start, sublayers, None)
}

pub(super) fn poc_lsb_aligned_with_hrd(
    bytes: &[u8],
    start: usize,
    sublayers: u8,
    base_hrd: Option<&super::hevc_hrd::Hrd>,
) -> Result<bool> {
    let mut b = BitReader::new(bytes);
    b.skip(start)?;
    while b.position() % 8 != 0 {
        if !b.bit()? {
            return Err(invalid("HEVC VPS extension alignment bit is not one"));
        }
    }
    // MaxLayersMinus1 == 0: no extra PTL, layer IDs, dependencies or OLSs.
    let splitting = b.bit()?;
    let mut count = 0u8;
    for _ in 0..16 {
        count += u8::from(b.bit()?);
    }
    if splitting && count == 0 {
        return Err(invalid("HEVC VPS splitting has no scalability dimensions"));
    }
    let mut offset = 0u32;
    for _ in 0..count - u8::from(splitting) {
        offset += b.read(3)? + 1;
    }
    if splitting && offset >= 6 {
        return Err(invalid("HEVC VPS split dimensions exceed layer ID"));
    }
    b.bit()?; // vps_nuh_layer_id_present_flag: no additional layer IDs
    let view_bits = b.read(4)? as u8;
    b.read(view_bits)?; // NumViews == 1
    let layer_sublayers = if b.bit()? {
        b.read(3)? as u8
    } else {
        sublayers
    };
    if layer_sublayers > sublayers {
        return Err(invalid("HEVC VPS extension exceeds temporal sublayers"));
    }
    b.bit()?; // max_tid_ref_present_flag: no dependency edges
    b.bit()?; // default_ref_layers_active_flag
    let profiles = bounded(&mut b, 63)?;
    for _ in 2..=profiles {
        let present = b.bit()?;
        ProfileTierLevel::read(&mut b, present, sublayers)?;
    }
    let formats = bounded(&mut b, 255)?;
    let mut previous = None;
    for index in 0..=formats {
        let width = b.read(16)?;
        let height = b.read(16)?;
        if width == 0 || height == 0 {
            return Err(invalid("zero HEVC VPS representation dimensions"));
        }
        let format = if b.bit()? {
            let chroma = b.read(2)?;
            let separate = chroma == 3 && b.bit()?;
            let luma = b.read(4)?;
            let chroma_depth = b.read(4)?;
            if luma > 8 || chroma_depth > 8 {
                return Err(invalid("HEVC VPS representation depth exceeds 16 bits"));
            }
            (chroma, separate)
        } else {
            previous
                .ok_or_else(|| invalid("first HEVC VPS representation format lacks chroma/depth"))?
        };
        previous = Some(format);
        if b.bit()? {
            let left = u64::from(b.unsigned_golomb()?);
            let right = u64::from(b.unsigned_golomb()?);
            let top = u64::from(b.unsigned_golomb()?);
            let bottom = u64::from(b.unsigned_golomb()?);
            let chroma = if format.1 { 0 } else { format.0 };
            let sx = if matches!(chroma, 1 | 2) { 2 } else { 1 };
            let sy = if chroma == 1 { 2 } else { 1 };
            if (left + right) * sx >= u64::from(width) || (top + bottom) * sy >= u64::from(height) {
                return Err(invalid("HEVC VPS representation crop removes picture"));
            }
        }
        debug_assert!(index <= 255);
    }
    if formats > 0 {
        b.bit()?;
    } // rep_format_idx_present_flag: base index inferred zero
    b.bit()?; // max_one_active_ref_layer_flag
    let aligned = b.bit()?;
    // No poc_lsb_not_present flags or additional-layer DPB entries.
    let dependency_width = bounded(&mut b, 30)? as u8 + 2;
    if b.bit()? && b.read(dependency_width)? == u32::MAX {
        return Err(invalid("invalid HEVC VPS direct dependency type"));
    }
    let ignored_bytes = bounded(&mut b, 4096)? as usize;
    b.skip(ignored_bytes * 8)?;
    if b.bit()? {
        while b.position() % 8 != 0 {
            if !b.bit()? {
                return Err(invalid("HEVC VPS VUI alignment bit is not one"));
            }
        }
        single_layer_vui(&mut b, layer_sublayers, sublayers, base_hrd)?;
    }
    if b.bit()? {
        while b.more_rbsp_data() {
            b.bit()?;
        }
    }
    b.finish_rbsp()?;
    Ok(aligned)
}

/// Annex F.7.3.2.1.4 with one internal layer/set/OLS. Advisory cross-layer,
/// rate and video-signal fields do not replace the base-layer SPS VUI (F.15.3.1).
fn single_layer_vui(
    b: &mut BitReader<'_>,
    layer_sublayers: u8,
    sublayers: u8,
    base_hrd: Option<&super::hevc_hrd::Hrd>,
) -> Result<()> {
    let irap_aligned = if b.bit()? { true } else { b.bit()? };
    if irap_aligned {
        b.bit()?;
    }
    let rate = b.bit()?;
    let pictures = b.bit()?;
    if rate || pictures {
        for _ in 0..=layer_sublayers {
            let has_rate = rate && b.bit()?;
            let has_pictures = pictures && b.bit()?;
            if has_rate {
                b.read(16)?;
                b.read(16)?;
            }
            if has_pictures {
                if b.read(2)? > 2 {
                    return Err(invalid("HEVC VPS VUI constant picture rate out of range"));
                }
                b.read(16)?;
            }
        }
    }
    let indexed = b.bit()?;
    let signals = if indexed { b.read(4)? } else { 0 };
    for _ in 0..=signals {
        b.read(3)?;
        b.bit()?;
        b.read(24)?;
    }
    if indexed && signals > 0 && b.read(4)? > signals {
        return Err(invalid("HEVC VPS VUI video signal index out of range"));
    }
    if !b.bit()? && b.bit()? {
        b.bit()?;
    } // tiles in the only layer
    if !b.bit()? {
        b.bit()?;
    } // WPP in the only layer
    let single = b.bit()?;
    if b.bit()? && !single {
        return Err(invalid(
            "HEVC VPS VUI higher-layer skip requires single-layer constraint",
        ));
    }
    b.bit()?; // ILP restrictions have no edges in a single-layer VPS
    if b.bit()? {
        let count = b.unsigned_golomb()?;
        let existing = u32::from(base_hrd.is_some());
        if count > 1024 - existing {
            return Err(invalid("HEVC VPS VUI additional HRD count out of range"));
        }
        let mut common = base_hrd.map(|h| h.common.clone());
        for index in existing..existing + count {
            let present = index == 0 || b.bit()?;
            let layers = b.unsigned_golomb()?;
            if layers > u32::from(sublayers) {
                return Err(invalid("HEVC VPS VUI HRD sublayers exceed VPS"));
            }
            let hrd = super::hevc_hrd::Hrd::read(
                b,
                layers as u8,
                if present { None } else { common.as_ref() },
            )?;
            common = Some(hrd.common);
        }
        // Output layer set zero has inferred partitioning; no schedule syntax.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ue(n: u32) -> String {
        let value = format!("{:b}", u64::from(n) + 1);
        format!("{}{}", "0".repeat(value.len() - 1), value)
    }
    fn packed(mut value: String) -> Vec<u8> {
        while value.len() % 8 != 0 {
            value.push('0');
        }
        value
            .as_bytes()
            .chunks(8)
            .map(|v| v.iter().fold(0, |a, c| (a << 1) | u8::from(*c == b'1')))
            .collect()
    }
    fn syntax(aligned: bool) -> String {
        // Three prefix bits, five alignment ones; no extra layers/OLSs/PTLs.
        format!(
            "00011111{}0000{}{:016b}{:016b}1010000000000{}101001",
            "0".repeat(18),
            "00011",
            64,
            64,
            u8::from(aligned)
        )
    }
    fn with_vui(vui: &str) -> Vec<u8> {
        let original = syntax(true);
        let mut prefix = original[..84].to_owned() + "1";
        while prefix.len() % 8 != 0 {
            prefix.push('1');
        }
        packed(prefix + vui + &original[85..])
    }
    fn basic_vui() -> String {
        "10000".to_owned() + "1010" + &"00000010".repeat(3) + "110000"
    }
    #[test]
    fn vui_optional_fields_truncation_and_hrd_inheritance_bounds() {
        let signal = "1010".to_owned() + &"00000010".repeat(3);
        let rate = "11".to_owned() + &"0000000000000001".repeat(2) + "10" + "0000000100000000";
        let rich = "0011".to_owned()
            + &rate.repeat(3)
            + "11111"
            + &signal.repeat(16)
            + "1111"
            + "011"
            + "01"
            + "101"
            + "0";
        let data = with_vui(&rich);
        assert!(poc_lsb_aligned(&data, 3, 2).unwrap());
        for end in 0..data.len() {
            assert!(poc_lsb_aligned(&data[..end], 3, 2).is_err(), "{end}");
        }
        let basic = basic_vui();
        assert!(poc_lsb_aligned(&with_vui(&basic), 3, 0).unwrap());
        let common_data = packed("00111".to_owned());
        let base =
            super::super::hevc_hrd::Hrd::read(&mut BitReader::new(&common_data), 0, None).unwrap();
        let inherited =
            basic[..basic.len() - 1].to_owned() + "1" + &ue(1023) + &"01111".repeat(1023);
        assert!(poc_lsb_aligned_with_hrd(&with_vui(&inherited), 3, 0, Some(&base)).unwrap());
        let too_many = basic[..basic.len() - 1].to_owned() + "1" + &ue(1024);
        assert_eq!(
            poc_lsb_aligned_with_hrd(&with_vui(&too_many), 3, 0, Some(&base))
                .unwrap_err()
                .to_string(),
            "HEVC VPS VUI additional HRD count out of range"
        );
        let independent = basic[..basic.len() - 1].to_owned()
            + "1"
            + &ue(1024)
            + "100111"
            + &"01111".repeat(1023);
        assert!(poc_lsb_aligned(&with_vui(&independent), 3, 0).unwrap());
    }
    #[test]
    fn single_layer_alignment_and_every_byte_truncation() {
        for aligned in [false, true] {
            let data = packed(syntax(aligned));
            assert_eq!(poc_lsb_aligned(&data, 3, 0).unwrap(), aligned);
            for end in 0..data.len() {
                assert!(poc_lsb_aligned(&data[..end], 3, 0).is_err(), "{end}");
            }
            let mut bad = data.clone();
            bad[0] &= !0x10;
            assert_eq!(
                poc_lsb_aligned(&bad, 3, 0).unwrap_err().to_string(),
                "HEVC VPS extension alignment bit is not one"
            );
        }
    }
    #[test]
    fn bounded_extension_counts_and_format_validation() {
        let prefix = format!("00011111{}0000000", "0".repeat(18));
        // Prefix ends before profile count. Larger-than-normative bounded counts
        // are Invalid, without allocating based on the signalled size.
        for (tail, message) in [
            (ue(64), "HEVC VPS extension value out of range"),
            (ue(0) + &ue(256), "HEVC VPS extension value out of range"),
            (
                ue(0) + &ue(0) + &format!("{:016b}{:016b}", 0, 64),
                "zero HEVC VPS representation dimensions",
            ),
            (
                ue(0) + &ue(0) + &format!("{:016b}{:016b}0", 64, 64),
                "first HEVC VPS representation format lacks chroma/depth",
            ),
        ] {
            let data = packed(prefix.clone() + &tail + "1");
            assert_eq!(
                poc_lsb_aligned(&data, 3, 0).unwrap_err().to_string(),
                message
            );
        }
        let split = packed("000111111".to_owned() + &"0".repeat(16) + "1");
        assert_eq!(
            poc_lsb_aligned(&split, 3, 0).unwrap_err().to_string(),
            "HEVC VPS splitting has no scalability dimensions"
        );
    }
    #[test]
    fn representation_inference_crop_depth_dependency_and_future_tail() {
        let original = syntax(true);
        let splice = |at: usize, remove: usize, value: &str| {
            packed(original[..at].to_owned() + value + &original[at + remove..])
        };
        for (data, message) in [
            (
                splice(70, 4, "1001"),
                "HEVC VPS representation depth exceeds 16 bits",
            ),
            (
                splice(78, 1, &("1".to_owned() + &ue(32) + &ue(0).repeat(3))),
                "HEVC VPS representation crop removes picture",
            ),
            (
                splice(81, 1, &ue(31)),
                "HEVC VPS extension value out of range",
            ),
            (
                splice(83, 1, &ue(4097)),
                "HEVC VPS extension value out of range",
            ),
        ] {
            assert_eq!(
                poc_lsb_aligned(&data, 3, 0).unwrap_err().to_string(),
                message
            );
        }
        assert_eq!(
            poc_lsb_aligned(&splice(84, 1, "1"), 3, 0)
                .unwrap_err()
                .to_string(),
            "HEVC VPS VUI alignment bit is not one"
        );
        assert!(poc_lsb_aligned(&splice(85, 1, "110110"), 3, 0).unwrap());
        assert!(poc_lsb_aligned(&splice(82, 1, "110"), 3, 0).unwrap());
        // Maximum 256 formats; later formats inherit chroma/depth, and the base
        // layer's representation index is inferred zero (no index bits).
        let mut many = original[..34].to_owned() + &ue(255) + &original[35..79];
        for _ in 1..256 {
            many += &format!("{:016b}{:016b}00", 64, 64);
        }
        many += "0"; // rep_format_idx_present_flag
        many += &original[79..];
        assert!(poc_lsb_aligned(&packed(many), 3, 0).unwrap());
    }
}
