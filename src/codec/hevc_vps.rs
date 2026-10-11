//! H.265 VPS topology/HRD metadata, strict base-layer admission and sublayer DPB limits.
pub use super::hevc_vps_extension::{BspHrdParameters, BspScheduleEntry, BspScheme};
pub use super::hevc_vps_extension::{DependencyTypes, LayeredExtensionPrefix};
pub use super::hevc_vps_extension::{DpbParameters, DpbSubLayer};
pub use super::hevc_vps_extension::{
    InterLayerRestriction, VideoSignalInfo, VpsVuiPrefix, VuiRate,
};
pub use super::hevc_vps_extension::{
    LayerDependencyPrefix, LayerSetExtension, OutputLayerSet, OutputLayerSets,
};
pub use super::hevc_vps_extension::{RepresentationFormat, RepresentationFormats};
use super::{bits::BitReader, hevc_nal::NalRbsp, hevc_profile::ProfileTierLevel};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ordering {
    pub max_decoded_pictures: u8,
    /// Per-SPS in Annex A, aggregate OLS reordering in Annex F.
    pub max_reorder_pictures: u32,
    pub max_latency_increase_plus1: u32,
}
/// Shared by VPS and SPS. Level/geometry-dependent DPB limits need a later SPS
/// conformance check; this reader enforces the absolute 16-picture bound.
pub fn read_ordering(bits: &mut BitReader<'_>, max_sub_layers_minus1: u8) -> Result<Vec<Ordering>> {
    if max_sub_layers_minus1 > 6 {
        return Err(invalid("invalid HEVC ordering sublayer count"));
    }
    let all = bits.bit()?;
    let count = usize::from(max_sub_layers_minus1) + 1;
    let mut output = vec![Ordering::default(); count];
    for i in if all { 0 } else { count - 1 }..count {
        let dpb_minus1 = bits.unsigned_golomb()?;
        let reorder = bits.unsigned_golomb()?;
        let latency = bits.unsigned_golomb()?;
        if dpb_minus1 > 15 || reorder > dpb_minus1 || latency == u32::MAX {
            return Err(invalid("invalid HEVC decoded-picture ordering limits"));
        }
        output[i] = Ordering {
            max_decoded_pictures: (dpb_minus1 + 1) as u8,
            max_reorder_pictures: reorder,
            max_latency_increase_plus1: latency,
        };
        if all
            && i > 0
            && (output[i].max_decoded_pictures < output[i - 1].max_decoded_pictures
                || output[i].max_reorder_pictures < output[i - 1].max_reorder_pictures)
        {
            return Err(invalid("HEVC sublayer ordering limits decrease"));
        }
    }
    if !all {
        let highest = output[count - 1];
        output.fill(highest);
    }
    Ok(output)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub units_in_tick: u32,
    pub time_scale: u32,
    pub ticks_per_poc: Option<u64>,
}

/// Compact deferred Annex F result. Unknown Annex A tails are never retained.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ResetExtension {
    Aligned(bool),
    Invalid(String),
    Unsupported(String),
    Io(std::io::ErrorKind, String),
    Gpu(String),
}
impl ResetExtension {
    fn from_result(result: Result<bool>) -> Self {
        match result {
            Ok(aligned) => Self::Aligned(aligned),
            Err(crate::Error::Invalid(s)) => Self::Invalid(s),
            Err(crate::Error::Unsupported(s)) => Self::Unsupported(s),
            Err(crate::Error::Io(e)) => Self::Io(e.kind(), e.to_string()),
            Err(crate::Error::Gpu(s)) => Self::Gpu(s),
        }
    }
    fn result(&self) -> Result<bool> {
        match self {
            Self::Aligned(value) => Ok(*value),
            Self::Invalid(s) => Err(invalid(s)),
            Self::Unsupported(s) => Err(crate::unsupported(s)),
            Self::Io(kind, s) => Err(crate::Error::Io(std::io::Error::new(*kind, s.clone()))),
            Self::Gpu(s) => Err(crate::Error::Gpu(s.clone())),
        }
    }
}

/// H.265 7.4.3.1 layer-set topology, including the inferred base-only set zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerSets {
    pub base_internal: bool,
    pub base_available: bool,
    /// Coded value, including reserved value 63 which decoders must allow.
    pub max_layers_minus1: u8,
    pub max_layer_id: u8,
    pub sets: Vec<u64>,
}
impl LayerSets {
    /// Normative MaxLayersMinus1 = Min(62, coded value), including future value 63.
    pub fn effective_max_layers(&self) -> u8 {
        self.max_layers_minus1.min(62) + 1
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerSetHrd {
    pub layer_set_index: u16,
    pub common_parameters_present: bool,
    pub parameters: super::hevc_hrd::Hrd,
}
fn hrd_bytes(h: &super::hevc_hrd::Hrd) -> usize {
    h.layers.capacity() * std::mem::size_of::<super::hevc_hrd::SubLayer>()
        + h.layers
            .iter()
            .map(|l| {
                (l.nal.capacity() + l.vcl.capacity()) * std::mem::size_of::<super::hevc_hrd::Cpb>()
            })
            .sum::<usize>()
}
#[derive(Clone, Debug)]
pub struct Vps {
    pub id: u8,
    pub layers: LayerSets,
    pub layer_hrd: Vec<LayerSetHrd>,
    pub temporal_id_nesting: bool,
    pub profile: ProfileTierLevel,
    pub ordering: Vec<Ordering>,
    #[cfg(test)]
    pub(crate) ordering_bit_range: std::ops::Range<usize>,
    pub timing: Option<Timing>,
    pub hrd: Option<super::hevc_hrd::Hrd>,
    /// Annex A may ignore this tail; Annex F reset decoding needs its semantics.
    pub extension_present: bool,
    reset_extension: Option<std::sync::Arc<ResetExtension>>,
    extension_start: Option<usize>,
    metadata_bytes: usize,
}
/// Complete VPS NAL metadata syntax through RBSP trailing bits. This does not
/// establish SPS/profile conformance or admit multilayer picture decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayeredVps {
    pub vps: Vps,
    pub extension: Option<Box<LayeredExtensionPrefix>>,
    pub vui: Option<Box<VpsVuiPrefix>>,
    pub bsp: Option<Box<BspHrdParameters>>,
    pub future_extension: bool,
}
impl LayeredVps {
    /// Resolve the PTL applying to a necessary layer of the selected OLS.
    /// Annex F.7.4.4 inherits missing general and sublayer fields from the
    /// preceding VPS PTL, independently of the selected layer's PTL index.
    /// This selects metadata; it does not certify a profile's coding tools.
    pub fn profile_for_layer(&self, ols: usize, layer_id: u8) -> Result<ProfileTierLevel> {
        let Some(extension) = self.extension.as_ref() else {
            if ols != 0 || layer_id != 0 || !self.vps.layers.base_internal {
                return Err(invalid("HEVC profile selection is outside the VPS"));
            }
            return Ok(self.vps.profile.clone());
        };
        let set = extension
            .output
            .sets
            .get(ols)
            .ok_or_else(|| invalid("HEVC profile output layer set is absent"))?;
        let layers = extension
            .layer_sets
            .layer_sets
            .get(set.layer_set_index)
            .ok_or_else(|| invalid("HEVC profile layer set is absent"))?;
        let position = layers
            .iter()
            .position(|&id| id == layer_id)
            .ok_or_else(|| invalid("HEVC profile layer is absent from the selected set"))?;
        let index = set
            .profile_indices
            .get(position)
            .copied()
            .flatten()
            .ok_or_else(|| {
                invalid("HEVC profile layer is not necessary for the selected output")
            })?;
        if index >= extension.output.profile_count || index > 63 {
            return Err(invalid("HEVC profile index is outside the VPS"));
        }
        if index == 0 && !self.vps.layers.base_internal {
            return Err(invalid("external HEVC base profile must be ignored"));
        }
        let count = self.vps.profile.sub_layers.len();
        if count > 6 {
            return Err(invalid("invalid HEVC profile sublayer count"));
        }
        let mut resolved = self.vps.profile.clone();
        if !self.vps.layers.base_internal {
            // F.7.4.4: external-base PTL0 bits are ignored. An absent PTL1
            // profile must not inherit an internal profile accidentally
            // retained by a permissive metadata reader.
            resolved.profile = Some(super::hevc_profile::Profile {
                space: 0,
                tier: false,
                idc: 0,
                compatibility: 0,
                constraints: 0,
            });
            resolved.level = 0;
            resolved
                .sub_layers
                .fill(super::hevc_profile::SubLayer::default());
        }
        for i in 1..=index {
            let coded = if i == 1 && self.vps.layers.base_internal {
                extension.graph.base_additional_profile.as_ref()
            } else {
                extension
                    .output
                    .extra_profiles
                    .iter()
                    .find(|(id, _)| *id == i)
                    .map(|(_, p)| p)
            }
            .ok_or_else(|| invalid("HEVC inherited profile predecessor is absent"))?;
            if coded.sub_layers.len() != count {
                return Err(invalid("HEVC inherited profile sublayer counts differ"));
            }
            resolved.profile = coded.profile.or(resolved.profile);
            resolved.level = coded.level;
            for (effective, field) in resolved.sub_layers.iter_mut().zip(&coded.sub_layers) {
                effective.profile = field.profile.or(effective.profile);
                effective.level = field.level.or(effective.level);
            }
        }
        Ok(resolved)
    }
    pub fn parse(nal: &[u8], budget: usize) -> Result<Self> {
        fn charge(left: &mut usize, used: usize) -> Result<()> {
            *left = left
                .checked_sub(used)
                .ok_or_else(|| invalid("HEVC layered VPS metadata exceeds memory budget"))?;
            Ok(())
        }
        let rbsp = NalRbsp::parse(nal, budget)?;
        let mut vps = Vps::parse_rbsp(&rbsp, budget, true)?;
        let mut left = budget;
        charge(
            &mut left,
            vps.metadata_bytes + std::mem::size_of::<Self>() - std::mem::size_of::<Vps>(),
        )?;
        let mut extension = None;
        let mut vui = None;
        let mut bsp = None;
        let mut future = false;
        if let Some(start) = vps.extension_start {
            let maximum = vps.ordering.len() as u8 - 1;
            let prefix =
                LayeredExtensionPrefix::parse(&rbsp.bytes, start, &vps.layers, maximum, left)?;
            charge(&mut left, prefix.retained_bytes())?;
            let mut end = prefix.dependencies.end_bit;
            if let Some(parsed) = prefix.vui_prefix(&rbsp.bytes, &vps.layers, left)? {
                charge(&mut left, parsed.retained_bytes())?;
                end = parsed.end_bit;
                let reference_bytes =
                    vps.layer_hrd.len() * std::mem::size_of::<&super::hevc_hrd::Hrd>();
                if left < reference_bytes {
                    return Err(invalid("HEVC layered VPS metadata exceeds memory budget"));
                }
                let references: Vec<_> = vps.layer_hrd.iter().map(|h| &h.parameters).collect();
                if let Some(parameters) = parsed.bsp_hrd_refs(
                    &rbsp.bytes,
                    &prefix,
                    &vps.layers,
                    &references,
                    maximum,
                    vps.timing.is_some(),
                    left - reference_bytes,
                )? {
                    charge(&mut left, parameters.retained_bytes())?;
                    end = parameters.end_bit;
                    bsp = Some(Box::new(parameters));
                }
                vui = Some(Box::new(parsed));
            }
            let mut bits = BitReader::new(&rbsp.bytes);
            bits.skip(end)?;
            future = bits.bit()?;
            if future {
                while bits.more_rbsp_data() {
                    bits.bit()?;
                }
            }
            bits.finish_rbsp()?;
            // Replacement allocation briefly overlaps the deferred Annex A
            // witness, whose storage was charged with the base VPS.
            charge(
                &mut left,
                std::mem::size_of::<ResetExtension>() + 2 * std::mem::size_of::<usize>(),
            )?;
            vps.reset_extension = Some(std::sync::Arc::new(ResetExtension::Aligned(
                prefix.representations.poc_lsb_aligned,
            )));
            extension = Some(Box::new(prefix));
        } else if vps.layers.effective_max_layers() > 1 {
            return Err(invalid("multilayer HEVC VPS requires extension syntax"));
        }
        Ok(Self {
            vps,
            extension,
            vui,
            bsp,
            future_extension: future,
        })
    }
}
// Annex A ignores extension data. Keep the admission witness available for
// Annex F without turning an inert tail into an ordinary VPS semantic change.
impl PartialEq for Vps {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.layers == other.layers
            && self.layer_hrd == other.layer_hrd
            && self.temporal_id_nesting == other.temporal_id_nesting
            && self.profile == other.profile
            && self.ordering == other.ordering
            && self.timing == other.timing
            && self.hrd == other.hrd
    }
}
impl Eq for Vps {}
impl Vps {
    pub(super) fn same_reset_semantics(&self, other: &Self) -> bool {
        self.reset_extension
            .as_deref()
            .unwrap_or(&ResetExtension::Aligned(false))
            == other
                .reset_extension
                .as_deref()
                .unwrap_or(&ResetExtension::Aligned(false))
    }
    /// A successfully interpreted Annex F tail activates POC cycle syntax even
    /// when the PPS does not signal reset IDC. Unknown Annex A future tails
    /// remain opaque until explicitly required by reset semantics.
    pub(super) fn has_poc_extension(&self) -> bool {
        matches!(self.reset_extension.as_deref(), Some(ResetExtension::Aligned(_)))
    }
    /// Strict Annex F semantics, separate from Annex A's ignored future tail.
    pub fn poc_lsb_aligned(&self) -> Result<bool> {
        self.reset_extension
            .as_ref()
            .map_or(Ok(false), |value| value.result())
    }
    pub fn parse(nal: &[u8], budget: usize) -> Result<Self> {
        Self::parse_internal(nal, budget, false)
    }
    /// Read layer-set and HRD metadata without claiming multilayer picture support.
    /// The retained metadata is budgeted in addition to the unescaped RBSP;
    /// reading one HRD uses bounded temporary storage (7 sublayers, 32 CPBs/list).
    pub fn parse_layered_metadata(nal: &[u8], budget: usize) -> Result<Self> {
        Self::parse_internal(nal, budget, true)
    }
    fn parse_internal(nal: &[u8], budget: usize, layered: bool) -> Result<Self> {
        let rbsp = NalRbsp::parse(nal, budget)?;
        Self::parse_rbsp(&rbsp, budget, layered)
    }
    fn parse_rbsp(rbsp: &NalRbsp, budget: usize, layered: bool) -> Result<Self> {
        rbsp.header.require_base_layer()?;
        if rbsp.header.unit_type != 32 || rbsp.header.temporal_id != 0 {
            return Err(invalid("expected base-layer HEVC VPS at temporal ID zero"));
        }
        let mut bits = BitReader::new(&rbsp.bytes);
        let id = bits.read(4)? as u8;
        let internal = bits.bit()?;
        let available = bits.bit()?;
        let max_layers_minus1 = bits.read(6)? as u8;
        let max_sub_layers_minus1 = bits.read(3)? as u8;
        let temporal_id_nesting = bits.bit()?;
        if !layered && (!internal || !available || max_layers_minus1 != 0) {
            return Err(crate::unsupported(
                "multilayer or external-base HEVC VPS is not implemented",
            ));
        }
        if !internal && max_layers_minus1 == 0 {
            return Err(invalid("external HEVC base layer requires multiple layers"));
        }
        if max_sub_layers_minus1 == 0 && !temporal_id_nesting {
            return Err(invalid("single-sublayer VPS requires temporal nesting"));
        }
        // H.265 7.4.3.1 explicitly requires decoders to ignore this reserved value.
        bits.skip(16)?;
        let profile = ProfileTierLevel::read(&mut bits, true, max_sub_layers_minus1)?;
        #[cfg(test)]
        let ordering_start = bits.position();
        let ordering = read_ordering(&mut bits, max_sub_layers_minus1)?;
        #[cfg(test)]
        let ordering_bit_range = ordering_start..bits.position();
        let max_layer_id = bits.read(6)? as u8;
        let extra_sets = bits.unsigned_golomb()?;
        if !layered && (max_layer_id != 0 || extra_sets != 0) {
            return Err(crate::unsupported(
                "multiple HEVC layer sets are not implemented",
            ));
        }
        if extra_sets > 1023 {
            return Err(invalid("HEVC VPS layer set count exceeds 1024"));
        }
        let mut metadata_bytes = rbsp.bytes.capacity()
            + std::mem::size_of::<Self>()
            + (extra_sets as usize + 1) * 8
            + profile.sub_layers.capacity() * std::mem::size_of::<super::hevc_profile::SubLayer>()
            + ordering.capacity() * std::mem::size_of::<Ordering>();
        if layered && metadata_bytes > budget {
            return Err(invalid("HEVC VPS metadata exceeds memory budget"));
        }
        let mut sets = Vec::with_capacity(extra_sets as usize + 1);
        sets.push(1);
        for _ in 0..extra_sets {
            let mut mask = 0u64;
            for id in 0..=max_layer_id {
                if bits.bit()? {
                    mask |= 1u64 << id;
                }
            }
            if mask == 0 || mask.count_ones() > u32::from(max_layers_minus1) + 1 {
                return Err(invalid("invalid HEVC VPS layer set cardinality"));
            }
            if sets.contains(&mask) {
                return Err(invalid("duplicate HEVC VPS layer set"));
            }
            sets.push(mask);
        }
        let layers = LayerSets {
            base_internal: internal,
            base_available: available,
            max_layers_minus1,
            max_layer_id,
            sets,
        };
        let mut layer_hrd: Vec<LayerSetHrd> = Vec::new();
        let mut hrd = None;
        let timing = if bits.bit()? {
            let units_in_tick = bits.read(32)?;
            let time_scale = bits.read(32)?;
            if units_in_tick == 0 || time_scale == 0 {
                return Err(invalid("zero HEVC VPS clock"));
            }
            let ticks_per_poc = if bits.bit()? {
                Some(u64::from(
                    bits.unsigned_golomb()?
                        .checked_add(1)
                        .ok_or_else(|| invalid("VPS POC tick count exceeds range"))?,
                ))
            } else {
                None
            };
            let count = bits.unsigned_golomb()? as usize;
            if count > layers.sets.len() {
                return Err(invalid("VPS HRD count exceeds layer sets"));
            }
            metadata_bytes += count * std::mem::size_of::<LayerSetHrd>();
            if layered && metadata_bytes > budget {
                return Err(invalid("HEVC VPS metadata exceeds memory budget"));
            }
            layer_hrd
                .try_reserve_exact(count)
                .map_err(|_| invalid("cannot allocate HEVC VPS HRD metadata"))?;
            for i in 0..count {
                let index = bits.unsigned_golomb()?;
                if index as usize >= layers.sets.len() || (!internal && index == 0) {
                    return Err(invalid("invalid VPS HRD layer set index"));
                }
                if layer_hrd
                    .iter()
                    .any(|h| u32::from(h.layer_set_index) == index)
                {
                    return Err(invalid("duplicate VPS HRD layer set index"));
                }
                let present = i == 0 || bits.bit()?;
                let inherited = if present {
                    None
                } else {
                    Some(&layer_hrd[i - 1].parameters.common)
                };
                let parameters =
                    super::hevc_hrd::Hrd::read(&mut bits, max_sub_layers_minus1, inherited)?;
                metadata_bytes = metadata_bytes
                    .checked_add(hrd_bytes(&parameters))
                    .ok_or_else(|| invalid("HEVC VPS metadata size overflow"))?;
                if index == 0 {
                    metadata_bytes += hrd_bytes(&parameters);
                }
                if layered && metadata_bytes > budget {
                    return Err(invalid("HEVC VPS metadata exceeds memory budget"));
                }
                if index == 0 {
                    hrd = Some(parameters.clone());
                }
                layer_hrd.push(LayerSetHrd {
                    layer_set_index: index as u16,
                    common_parameters_present: present,
                    parameters,
                });
            }
            Some(Timing {
                units_in_tick,
                time_scale,
                ticks_per_poc,
            })
        } else {
            None
        };
        let extension_present = bits.bit()?;
        let extension_start = extension_present.then_some(bits.position());
        let reset_extension = extension_present.then(|| {
            std::sync::Arc::new(
                if max_layers_minus1 != 0 || max_layer_id != 0 || extra_sets != 0 {
                    ResetExtension::Unsupported(
                        "HEVC multilayer VPS extension dependencies are not connected".into(),
                    )
                } else {
                    ResetExtension::from_result(
                        super::hevc_vps_extension::poc_lsb_aligned_with_hrd(
                            &rbsp.bytes,
                            bits.position(),
                            max_sub_layers_minus1,
                            hrd.as_ref(),
                        ),
                    )
                },
            )
        });
        if layered {
            if let Some(extension) = reset_extension.as_deref() {
                metadata_bytes +=
                    std::mem::size_of::<ResetExtension>() + 2 * std::mem::size_of::<usize>();
                metadata_bytes += match extension {
                    ResetExtension::Invalid(s)
                    | ResetExtension::Unsupported(s)
                    | ResetExtension::Gpu(s)
                    | ResetExtension::Io(_, s) => s.capacity(),
                    ResetExtension::Aligned(_) => 0,
                };
            }
            if metadata_bytes > budget {
                return Err(invalid("HEVC VPS metadata exceeds memory budget"));
            }
        }
        if extension_present {
            // Annex A decoding without INBLD ignores VPS extension data.
            // Base-layer admission above still rejects layered requirements.
            while bits.more_rbsp_data() {
                bits.bit()?;
            }
        }
        bits.finish_rbsp()?;
        Ok(Self {
            id,
            layers,
            layer_hrd,
            temporal_id_nesting,
            profile,
            ordering,
            extension_present,
            reset_extension,
            extension_start,
            metadata_bytes,
            #[cfg(test)]
            ordering_bit_range,
            timing,
            hrd,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ordering_bits(all: bool, values: &[[u32; 3]]) -> Vec<u8> {
        let mut bits = vec![all];
        for value in values.iter().flatten() {
            let code = u64::from(*value) + 1;
            let width = 64 - code.leading_zeros();
            bits.extend(std::iter::repeat_n(false, (width - 1) as usize));
            for shift in (0..width).rev() {
                bits.push(code & (1 << shift) != 0);
            }
        }
        bits.push(true);
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        bits.chunks(8)
            .map(|chunk| {
                chunk
                    .iter()
                    .fold(0, |byte, &bit| (byte << 1) | u8::from(bit))
            })
            .collect()
    }
    #[test]
    fn ordering_inference_monotonicity_and_bounds() {
        let data = ordering_bits(false, &[[4, 2, 5]]);
        let mut bits = BitReader::new(&data);
        let limits = read_ordering(&mut bits, 2).unwrap();
        assert_eq!(
            limits,
            vec![
                Ordering {
                    max_decoded_pictures: 5,
                    max_reorder_pictures: 2,
                    max_latency_increase_plus1: 5
                };
                3
            ]
        );
        bits.finish_rbsp().unwrap();
        let valid = ordering_bits(true, &[[1, 0, 0], [4, 2, 5]]);
        assert_eq!(
            read_ordering(&mut BitReader::new(&valid), 1).unwrap()[0].max_decoded_pictures,
            2
        );
        for invalid_values in [
            vec![[16, 0, 0]],
            vec![[1, 2, 0]],
            vec![[1, 0, u32::MAX]],
            vec![[4, 2, 0], [3, 2, 0]],
            vec![[4, 2, 0], [4, 1, 0]],
        ] {
            let data = ordering_bits(true, &invalid_values);
            assert!(
                read_ordering(&mut BitReader::new(&data), (invalid_values.len() - 1) as u8)
                    .is_err()
            );
        }
    }
    #[test]
    fn real_vps_and_every_truncation() {
        let hex = "40010c01ffff01600000030090000003000003001e959809";
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let vps = Vps::parse(&bytes, 1024).unwrap();
        assert_eq!(vps.id, 0);
        assert_eq!(vps.profile.profile.unwrap().idc, 1);
        assert_eq!(
            vps.ordering,
            vec![Ordering {
                max_decoded_pictures: 5,
                max_reorder_pictures: 2,
                max_latency_increase_plus1: 5
            }]
        );
        assert_eq!(vps.timing, None);
        for end in 0..bytes.len() {
            assert!(Vps::parse(&bytes[..end], 1024).is_err());
        }
        assert!(Vps::parse(&bytes, 1).is_err());
        let mut extra = bytes.clone();
        extra.push(0x80);
        assert!(Vps::parse(&extra, 1024).is_err());
    }
}
