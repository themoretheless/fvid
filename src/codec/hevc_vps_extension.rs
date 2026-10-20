//! Annex F dependency/layer-set prefixes and strict single-layer VPS extension.
//! Ordinary Annex A may ignore errors in the deferred single-layer tail.
use super::{bits::BitReader, hevc_profile::ProfileTierLevel};
use crate::{Result, invalid};
#[path = "hevc_bsp_hrd.rs"]
mod bsp;
pub use bsp::{BspHrdParameters, BspScheduleEntry, BspScheme};

/// Annex F extension syntax through direct_dependency_flag, before additional
/// sets/OLS/DPB. Masks use NAL layer IDs, not VPS indices. This is deliberately
/// a prefix reader, not admission of a complete multilayer picture stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerDependencyPrefix {
    /// VpsProfileTierLevel[1] when the internal base layer has an extension.
    /// Profile fields absent here inherit from VpsProfileTierLevel[0].
    pub base_additional_profile: Option<ProfileTierLevel>,
    pub layer_ids: Vec<u8>,
    pub scalability_ids: Vec<[u8; 16]>,
    pub view_ids: Vec<u16>,
    pub direct: Vec<u64>,
    pub transitive: Vec<u64>,
    pub tree_partitions: Vec<Vec<u8>>,
    pub end_bit: usize,
}
impl LayerDependencyPrefix {
    /// Annex I ViewIdx is scalability dimension 1, not nuh_layer_id and not
    /// view_id_val (whose values may be equal or unrelated to layer ordering).
    pub fn view_index(&self, layer_id: u8) -> Result<u16> {
        let _ = self.depth_layer_ids()?;
        let index = self.layer_ids.iter().position(|&id|id==layer_id)
            .ok_or_else(|| invalid("HEVC view index names an unknown layer"))?;
        Ok(u16::from(self.scalability_ids[index][1]))
    }
    /// I-11 ViewIdVal(pic): numeric view identifier, distinct from ViewIdx.
    pub fn view_id_value(&self, layer_id: u8) -> Result<u16> {
        let index=usize::from(self.view_index(layer_id)?);
        let value=self.view_ids.get(index).copied()
            .ok_or_else(||invalid("HEVC view index exceeds view identifier table"))?;
        if value>32767 {return Err(invalid("HEVC view identifier exceeds coded bit width"));}
        Ok(value)
    }
    /// I-8 reference-list layers share the current layer's depth/texture role.
    /// Opposite-role dependencies remain in `direct` for AU ordering and tools.
    pub fn reference_list_layers(&self,layer_id:u8)->Result<u64> {
        let _=self.depth_layer_ids()?;
        if self.direct.len()!=self.layer_ids.len() {
            return Err(invalid("invalid HEVC reference-list layer graph"));
        }
        let index=self.layer_ids.iter().position(|&id|id==layer_id)
            .ok_or_else(||invalid("HEVC reference list names an unknown layer"))?;
        let lower=self.layer_ids[..index].iter().fold(0u64,|mask,&id|mask|(1u64<<id));
        if self.direct[index]&!lower!=0 {return Err(invalid("invalid HEVC reference-list dependency"));}
        Ok(self.layer_ids.iter().copied().zip(&self.scalability_ids)
            .filter(|(_,d)|d[0]==self.scalability_ids[index][0])
            .fold(0u64,|mask,(id,_)|mask|((1u64<<id)&self.direct[index])))
    }
    /// I-9 ViewCompLayerId: identify a view component, not its AU availability.
    /// Layer IDs are resolved from scalability dimensions, never view_id_val.
    pub fn view_component_layer(&self, layer_id:u8, depth:bool)->Result<Option<u8>> {
        let _=self.depth_layer_ids()?;
        let index=self.layer_ids.iter().position(|&id|id==layer_id)
            .ok_or_else(||invalid("HEVC view component names an unknown layer"))?;
        self.view_component_for_index(u16::from(self.scalability_ids[index][1]),depth)
    }
    /// I-9 mapping for an explicit ViewIdx. This is not a numeric view_id_val
    /// or a NAL layer ID. Dependency/auxiliary representations are excluded.
    pub fn view_component_for_index(&self,view_index:u16,depth:bool)->Result<Option<u8>> {
        let _=self.depth_layer_ids()?;
        Ok(self.layer_ids.iter().copied().zip(&self.scalability_ids)
            .filter(|(_,d)|d[0]==u8::from(depth) && u16::from(d[1])==view_index && d[2]==0 && d[3]==0)
            .map(|(id,_)|id).last())
    }

    /// F-3 / I-13 depth-layer order, preserving VPS order rather than a bit mask.
    pub fn depth_layer_ids(&self) -> Result<impl Iterator<Item = u8> + '_> {
        if self.layer_ids.is_empty() || self.layer_ids.len() > 63
            || self.layer_ids.len() != self.scalability_ids.len()
            || self.layer_ids[0] != 0
            || self.layer_ids.iter().any(|id| *id > 62)
            || self.layer_ids.windows(2).any(|v| v[0] >= v[1])
            || self.scalability_ids.iter().any(|d| d[0] > 1)
            || self.scalability_ids[0][0] != 0
        {
            return Err(invalid("invalid HEVC VPS depth-layer identity graph"));
        }
        Ok(self.layer_ids.iter().copied().zip(&self.scalability_ids)
            .filter_map(|(id, dimensions)| (dimensions[0] == 1).then_some(id)))
    }
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepresentationFormat {
    pub width: u16,
    pub height: u16,
    pub chroma: u8,
    pub separate_colour_planes: bool,
    pub luma_depth: u8,
    pub chroma_depth: u8,
    /// Coded conformance offsets, in units determined by ChromaArrayType.
    pub crop: [u32; 4],
}
impl RepresentationFormat {
    pub fn display_size(&self) -> Result<(u16, u16)> {
        let cat = if self.separate_colour_planes {
            0
        } else {
            self.chroma
        };
        let sx = if matches!(cat, 1 | 2) { 2u64 } else { 1 };
        let sy = if cat == 1 { 2u64 } else { 1 };
        let x = (u64::from(self.crop[0]) + u64::from(self.crop[1])) * sx;
        let y = (u64::from(self.crop[2]) + u64::from(self.crop[3])) * sy;
        if x >= u64::from(self.width) || y >= u64::from(self.height) {
            return Err(invalid("HEVC VPS representation crop removes picture"));
        }
        Ok((
            (u64::from(self.width) - x) as u16,
            (u64::from(self.height) - y) as u16,
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepresentationFormats {
    pub formats: Vec<RepresentationFormat>,
    /// VPS layer-index order, including the inferred base mapping.
    pub layer_format_indices: Vec<u8>,
    pub max_one_active_reference: bool,
    pub poc_lsb_aligned: bool,
    pub poc_lsb_not_present: Vec<bool>,
    pub end_bit: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpbSubLayer {
    pub explicitly_present: bool,
    /// Actual picture counts, in associated layer-list order; None if unnecessary.
    pub decoded_pictures: Vec<Option<u32>>,
    pub reorder_pictures: u32,
    pub latency_increase_plus1: u32,
}
impl DpbSubLayer {
    pub fn max_latency_pictures(&self) -> Option<u64> {
        (self.latency_increase_plus1 != 0)
            .then(|| u64::from(self.reorder_pictures) + u64::from(self.latency_increase_plus1) - 1)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpbParameters {
    /// OLS zero is None: its DPB is supplied by the base SPS, not this syntax.
    pub sets: Vec<Option<Vec<DpbSubLayer>>>,
    pub end_bit: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyTypes {
    /// VPS indices [dependent][reference], None where no direct edge exists.
    pub types: Vec<Vec<Option<u32>>>,
    /// Masks use NAL layer IDs.
    pub sample_prediction: Vec<u64>,
    pub motion_prediction: Vec<u64>,
    pub coded_width: u8,
    pub common_type: Option<u32>,
    pub ignored_extension_bytes: u16,
    pub vui_present: bool,
    /// Start of VUI syntax if present; otherwise the end of vps_extension().
    pub end_bit: usize,
}
impl DpbParameters {
    pub fn dependency_types(
        &self,
        bytes: &[u8],
        graph: &LayerDependencyPrefix,
        layers: &super::hevc_vps::LayerSets,
        budget: usize,
    ) -> Result<DependencyTypes> {
        let n = graph.layer_ids.len();
        if n == 0 || n > 63 || graph.direct.len() != n || graph.layer_ids.iter().any(|&id| id > 63)
        {
            return Err(invalid("invalid HEVC dependency type geometry"));
        }
        if std::mem::size_of::<DependencyTypes>()
            + n * (n * std::mem::size_of::<Option<u32>>()
                + std::mem::size_of::<Vec<Option<u32>>>()
                + 16)
            > budget
        {
            return Err(invalid("HEVC dependency types exceed memory budget"));
        }
        let mut b = BitReader::new(bytes);
        b.skip(self.end_bit)?;
        let width = bounded(&mut b, 30)? as u8 + 2;
        let common = if b.bit()? { Some(b.read(width)?) } else { None };
        if common == Some(u32::MAX) {
            return Err(invalid("invalid HEVC direct dependency type"));
        }
        let mut types = vec![vec![None; n]; n];
        let mut sample = vec![0u64; n];
        let mut motion = vec![0u64; n];
        for i in 1..n {
            for j in 0..i {
                if graph.direct[i] & (1u64 << graph.layer_ids[j]) == 0 {
                    continue;
                }
                let value = if !layers.base_internal && j == 0 {
                    0
                } else if let Some(value) = common {
                    value
                } else {
                    b.read(width)?
                };
                if value == u32::MAX {
                    return Err(invalid("invalid HEVC direct dependency type"));
                }
                types[i][j] = Some(value);
                if (value + 1) & 1 != 0 {
                    sample[i] |= 1u64 << graph.layer_ids[j];
                }
                if (value + 1) & 2 != 0 {
                    motion[i] |= 1u64 << graph.layer_ids[j];
                }
            }
        }
        let ignored = bounded(&mut b, 4096)? as u16;
        b.skip(usize::from(ignored) * 8)?;
        let vui = b.bit()?;
        if vui {
            while b.position() % 8 != 0 {
                if !b.bit()? {
                    return Err(invalid("HEVC VPS VUI alignment bit is not one"));
                }
            }
        }
        Ok(DependencyTypes {
            types,
            sample_prediction: sample,
            motion_prediction: motion,
            coded_width: width,
            common_type: common,
            ignored_extension_bytes: ignored,
            vui_present: vui,
            end_bit: b.position(),
        })
    }
}
/// Connected Annex F prefix through dependency types/non-VUI extension bytes.
/// VPS VUI, SPS admission and multilayer picture reconstruction remain separate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayeredExtensionPrefix {
    pub graph: LayerDependencyPrefix,
    pub layer_sets: LayerSetExtension,
    pub output: OutputLayerSets,
    pub representations: RepresentationFormats,
    pub dpb: DpbParameters,
    pub dependencies: DependencyTypes,
    retained_bytes: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoSignalInfo {
    pub format: u8,
    pub full_range: bool,
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VuiRate {
    pub bit_rate: Option<(u16, u16)>,
    pub picture_rate: Option<(u8, u16)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterLayerRestriction {
    pub spatial_offset_plus1: u32,
    pub ctu_based: bool,
    pub horizontal_offset_plus1: u32,
}
/// General VPS VUI prefix. When BSP HRD is present, end_bit points to its syntax
/// and base-layer compatibility flags remain deferred until after BSP parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VpsVuiPrefix {
    pub picture_types_aligned: bool,
    pub irap_aligned: bool,
    pub all_idr_aligned: bool,
    pub rates: Vec<Option<Vec<VuiRate>>>,
    pub video_signals: Vec<VideoSignalInfo>,
    /// Raw inferred indices preserve the 06/2019 specification's layer-index
    /// rule; external-base non-indexed mapping is not yet qualified or applied.
    pub video_signal_indices: Vec<Option<u8>>,
    pub tiles_not_in_use: bool,
    pub tiles: Vec<bool>,
    pub loop_filter_not_across_tiles: Vec<bool>,
    pub aligned_tile_boundaries: Vec<u64>,
    pub wpp_not_in_use: bool,
    pub wpp: Vec<bool>,
    pub single_layer_non_irap: bool,
    pub higher_layer_irap_skip: bool,
    pub restrictions: Vec<Vec<Option<InterLayerRestriction>>>,
    pub bsp_hrd_present: bool,
    pub base_parameter_compatibility: Vec<Option<bool>>,
    pub end_bit: usize,
}
impl VpsVuiPrefix {
    pub fn retained_bytes(&self) -> usize {
        fn bytes<T>(v: &Vec<T>) -> usize {
            v.capacity() * std::mem::size_of::<T>()
        }
        std::mem::size_of::<Self>()
            + bytes(&self.rates)
            + self.rates.iter().flatten().map(bytes).sum::<usize>()
            + bytes(&self.video_signals)
            + bytes(&self.video_signal_indices)
            + bytes(&self.tiles)
            + bytes(&self.loop_filter_not_across_tiles)
            + bytes(&self.aligned_tile_boundaries)
            + bytes(&self.wpp)
            + bytes(&self.restrictions)
            + self.restrictions.iter().map(bytes).sum::<usize>()
            + bytes(&self.base_parameter_compatibility)
    }
}
impl LayeredExtensionPrefix {
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
    pub fn vui_prefix(
        &self,
        bytes: &[u8],
        layers: &super::hevc_vps::LayerSets,
        budget: usize,
    ) -> Result<Option<VpsVuiPrefix>> {
        if !self.dependencies.vui_present {
            return Ok(None);
        }
        let n = self.graph.layer_ids.len();
        if n <= usize::from(!layers.base_internal)
            || n > 63
            || self.graph.direct.len() != n
            || self.graph.layer_ids.iter().any(|&id| id > 63)
            || self.layer_sets.set_sublayers_minus1.len() > 2047
            || self.layer_sets.set_sublayers_minus1.iter().any(|&s| s > 6)
        {
            return Err(invalid("invalid HEVC VPS VUI geometry"));
        }
        let cost = 4096
            + n * n * std::mem::size_of::<Option<InterLayerRestriction>>()
            + self.layer_sets.set_sublayers_minus1.len()
                * (std::mem::size_of::<Option<Vec<VuiRate>>>()
                    + 7 * std::mem::size_of::<VuiRate>());
        if cost > budget {
            return Err(invalid("HEVC VPS VUI metadata exceeds memory budget"));
        }
        let mut b = BitReader::new(bytes);
        b.skip(self.dependencies.end_bit)?;
        let aligned = b.bit()?;
        let irap = aligned || b.bit()?;
        let idr = irap && b.bit()?;
        let has_rate = b.bit()?;
        let has_pictures = b.bit()?;
        let first = usize::from(!layers.base_internal);
        let mut rates = vec![None; self.layer_sets.set_sublayers_minus1.len()];
        if has_rate || has_pictures {
            for (i, entry) in rates.iter_mut().enumerate().skip(first) {
                let maximum = self.layer_sets.set_sublayers_minus1[i];
                let mut rows = Vec::with_capacity(usize::from(maximum) + 1);
                for _ in 0..=maximum {
                    let rate = has_rate && b.bit()?;
                    let pictures = has_pictures && b.bit()?;
                    let bit_rate = if rate {
                        Some((b.read(16)? as u16, b.read(16)? as u16))
                    } else {
                        None
                    };
                    let picture_rate = if pictures {
                        let constant = b.read(2)? as u8;
                        if constant > 2 {
                            return Err(invalid("HEVC VPS VUI constant picture rate out of range"));
                        }
                        Some((constant, b.read(16)? as u16))
                    } else {
                        None
                    };
                    rows.push(VuiRate {
                        bit_rate,
                        picture_rate,
                    });
                }
                *entry = Some(rows);
            }
        }
        let indexed = b.bit()?;
        let last = if indexed {
            b.read(4)? as usize
        } else {
            n - first - 1
        };
        let mut signals = Vec::with_capacity(last + 1);
        for _ in 0..=last {
            signals.push(VideoSignalInfo {
                format: b.read(3)? as u8,
                full_range: b.bit()?,
                primaries: b.read(8)? as u8,
                transfer: b.read(8)? as u8,
                matrix: b.read(8)? as u8,
            });
        }
        let mut indices = vec![None; n];
        for (i, index) in indices.iter_mut().enumerate().skip(first) {
            *index = Some(if indexed && last > 0 {
                let value = b.read(4)? as u8;
                if usize::from(value) > last {
                    return Err(invalid("HEVC VPS VUI video signal index out of range"));
                }
                value
            } else if indexed {
                0
            } else {
                i as u8
            });
        }
        let no_tiles = b.bit()?;
        let mut tiles = vec![false; n];
        let mut filter = vec![false; n];
        let mut tile_alignment = vec![0u64; n];
        if !no_tiles {
            for i in first..n {
                tiles[i] = b.bit()?;
                if tiles[i] {
                    filter[i] = b.bit()?;
                }
            }
            for i in first + 1..n {
                for j in 0..i {
                    if self.graph.direct[i] & (1u64 << self.graph.layer_ids[j]) != 0
                        && tiles[i]
                        && tiles[j]
                        && b.bit()?
                    {
                        tile_alignment[i] |= 1u64 << self.graph.layer_ids[j];
                    }
                }
            }
        }
        let no_wpp = b.bit()?;
        let mut wpp = vec![false; n];
        if !no_wpp {
            for value in wpp.iter_mut().skip(first) {
                *value = b.bit()?;
            }
        }
        let single = b.bit()?;
        let skip = b.bit()?;
        if skip && !single {
            return Err(invalid(
                "HEVC VPS VUI higher-layer skip requires single-layer constraint",
            ));
        }
        let mut restrictions = vec![vec![None; n]; n];
        if b.bit()? {
            for i in 1..n {
                for j in first..i {
                    if self.graph.direct[i] & (1u64 << self.graph.layer_ids[j]) == 0 {
                        continue;
                    }
                    let spatial = b.unsigned_golomb()?;
                    let ctu = spatial > 0 && b.bit()?;
                    let horizontal = if ctu { b.unsigned_golomb()? } else { 0 };
                    restrictions[i][j] = Some(InterLayerRestriction {
                        spatial_offset_plus1: spatial,
                        ctu_based: ctu,
                        horizontal_offset_plus1: horizontal,
                    });
                }
            }
        }
        let bsp = b.bit()?;
        let mut compatibility = vec![None; n];
        if !bsp {
            for (i, value) in compatibility.iter_mut().enumerate().skip(1) {
                if self.graph.direct[i] == 0 {
                    *value = Some(b.bit()?);
                }
            }
        }
        Ok(Some(VpsVuiPrefix {
            picture_types_aligned: aligned,
            irap_aligned: irap,
            all_idr_aligned: idr,
            rates,
            video_signals: signals,
            video_signal_indices: indices,
            tiles_not_in_use: no_tiles,
            tiles,
            loop_filter_not_across_tiles: filter,
            aligned_tile_boundaries: tile_alignment,
            wpp_not_in_use: no_wpp,
            wpp,
            single_layer_non_irap: single,
            higher_layer_irap_skip: skip,
            restrictions,
            bsp_hrd_present: bsp,
            base_parameter_compatibility: compatibility,
            end_bit: b.position(),
        }))
    }
}
impl LayeredExtensionPrefix {
    /// The budget covers all retained metadata together, excluding borrowed RBSP.
    pub fn parse(
        bytes: &[u8],
        start: usize,
        layers: &super::hevc_vps::LayerSets,
        sublayers: u8,
        budget: usize,
    ) -> Result<Self> {
        fn charge(left: &mut usize, used: usize) -> Result<()> {
            *left = left
                .checked_sub(used)
                .ok_or_else(|| invalid("HEVC layered metadata exceeds memory budget"))?;
            Ok(())
        }
        fn vec_bytes<T>(v: &Vec<T>) -> usize {
            v.capacity() * std::mem::size_of::<T>()
        }
        let mut left = budget;
        charge(
            &mut left,
            std::mem::size_of::<Self>()
                - std::mem::size_of::<LayerDependencyPrefix>()
                - std::mem::size_of::<LayerSetExtension>()
                - std::mem::size_of::<OutputLayerSets>()
                - std::mem::size_of::<RepresentationFormats>()
                - std::mem::size_of::<DpbParameters>()
                - std::mem::size_of::<DependencyTypes>(),
        )?;
        let graph = LayerDependencyPrefix::parse(bytes, start, layers, sublayers, left)?;
        charge(
            &mut left,
            std::mem::size_of::<LayerDependencyPrefix>()
                + vec_bytes(&graph.layer_ids)
                + vec_bytes(&graph.scalability_ids)
                + vec_bytes(&graph.view_ids)
                + vec_bytes(&graph.direct)
                + vec_bytes(&graph.transitive)
                + vec_bytes(&graph.tree_partitions)
                + graph
                    .base_additional_profile
                    .as_ref()
                    .map_or(0, |p| vec_bytes(&p.sub_layers))
                + graph.tree_partitions.iter().map(vec_bytes).sum::<usize>(),
        )?;
        let layer_sets = graph.layer_sets(bytes, layers, sublayers, left)?;
        charge(
            &mut left,
            std::mem::size_of::<LayerSetExtension>()
                + vec_bytes(&layer_sets.layer_sets)
                + layer_sets.layer_sets.iter().map(vec_bytes).sum::<usize>()
                + vec_bytes(&layer_sets.sublayers_minus1)
                + vec_bytes(&layer_sets.set_sublayers_minus1)
                + vec_bytes(&layer_sets.max_tid_ref_plus1)
                + layer_sets
                    .max_tid_ref_plus1
                    .iter()
                    .map(vec_bytes)
                    .sum::<usize>(),
        )?;
        let output = layer_sets.output_layers(bytes, &graph, layers, sublayers, left)?;
        charge(
            &mut left,
            std::mem::size_of::<OutputLayerSets>()
                + vec_bytes(&output.sets)
                + output
                    .sets
                    .iter()
                    .map(|s| vec_bytes(&s.profile_indices))
                    .sum::<usize>()
                + vec_bytes(&output.extra_profiles)
                + output
                    .extra_profiles
                    .iter()
                    .map(|(_, p)| vec_bytes(&p.sub_layers))
                    .sum::<usize>(),
        )?;
        let representations = output.representation_formats(bytes, &graph, layers, left)?;
        charge(
            &mut left,
            std::mem::size_of::<RepresentationFormats>()
                + vec_bytes(&representations.formats)
                + vec_bytes(&representations.layer_format_indices)
                + vec_bytes(&representations.poc_lsb_not_present),
        )?;
        let dpb = representations.dpb_parameters(bytes, &output, &layer_sets, layers, left)?;
        charge(
            &mut left,
            std::mem::size_of::<DpbParameters>()
                + vec_bytes(&dpb.sets)
                + dpb
                    .sets
                    .iter()
                    .flatten()
                    .map(|rows| {
                        vec_bytes(rows)
                            + rows
                                .iter()
                                .map(|r| vec_bytes(&r.decoded_pictures))
                                .sum::<usize>()
                    })
                    .sum::<usize>(),
        )?;
        let dependencies = dpb.dependency_types(bytes, &graph, layers, left)?;
        charge(
            &mut left,
            std::mem::size_of::<DependencyTypes>()
                + vec_bytes(&dependencies.types)
                + dependencies.types.iter().map(vec_bytes).sum::<usize>()
                + vec_bytes(&dependencies.sample_prediction)
                + vec_bytes(&dependencies.motion_prediction),
        )?;
        Ok(Self {
            graph,
            layer_sets,
            output,
            representations,
            dpb,
            dependencies,
            retained_bytes: budget - left,
        })
    }
}
impl RepresentationFormats {
    /// Read dpb_size(). Profile/level/SPS constraints and actual frame-storage
    /// admission remain separate from the retained metadata budget here.
    pub fn dpb_parameters(
        &self,
        bytes: &[u8],
        output: &OutputLayerSets,
        extension: &LayerSetExtension,
        layers: &super::hevc_vps::LayerSets,
        budget: usize,
    ) -> Result<DpbParameters> {
        if output.sets.is_empty()
            || output.sets.len() > 3070
            || extension.layer_sets.len() != extension.set_sublayers_minus1.len()
        {
            return Err(invalid("invalid HEVC DPB metadata geometry"));
        }
        let mut cost = std::mem::size_of::<DpbParameters>()
            + output.sets.len() * std::mem::size_of::<Option<Vec<DpbSubLayer>>>();
        for ols in output.sets.iter().skip(1) {
            let list = extension
                .layer_sets
                .get(ols.layer_set_index)
                .ok_or_else(|| invalid("HEVC DPB layer set index out of range"))?;
            let maximum = extension.set_sublayers_minus1[ols.layer_set_index];
            if maximum > 6 || list.is_empty() || list.len() > 63 || list.iter().any(|&id| id > 63) {
                return Err(invalid("invalid HEVC DPB layer geometry"));
            }
            cost += (usize::from(maximum) + 1)
                * (std::mem::size_of::<DpbSubLayer>()
                    + list.len() * std::mem::size_of::<Option<u32>>());
        }
        if cost > budget {
            return Err(invalid("HEVC DPB metadata exceeds memory budget"));
        }
        let mut b = BitReader::new(bytes);
        b.skip(self.end_bit)?;
        let mut sets = Vec::with_capacity(output.sets.len());
        sets.push(None);
        for ols in output.sets.iter().skip(1) {
            let list = &extension.layer_sets[ols.layer_set_index];
            let maximum = extension.set_sublayers_minus1[ols.layer_set_index];
            let flags = b.bit()?;
            let mut temporal: Vec<DpbSubLayer> = Vec::with_capacity(usize::from(maximum) + 1);
            for j in 0..=maximum {
                let present = j == 0 || (flags && b.bit()?);
                if !present {
                    let mut inherited = temporal.last().unwrap().clone();
                    inherited.explicitly_present = false;
                    temporal.push(inherited);
                    continue;
                }
                let mut pictures = vec![None; list.len()];
                for (k, &id) in list.iter().enumerate() {
                    if ols.necessary & (1u64 << id) == 0 {
                        continue;
                    }
                    let value = if layers.base_internal || id != 0 {
                        b.unsigned_golomb()?
                            .checked_add(1)
                            .ok_or_else(|| invalid("HEVC DPB picture count overflow"))?
                    } else {
                        1
                    };
                    if let Some(previous) = temporal.last().and_then(|p| p.decoded_pictures[k]) {
                        if value < previous {
                            return Err(invalid(
                                "HEVC DPB capacity decreases with temporal sublayer",
                            ));
                        }
                    }
                    pictures[k] = Some(value);
                }
                let reorder = b.unsigned_golomb()?;
                let latency = b.unsigned_golomb()?;
                temporal.push(DpbSubLayer {
                    explicitly_present: true,
                    decoded_pictures: pictures,
                    reorder_pictures: reorder,
                    latency_increase_plus1: latency,
                });
            }
            sets.push(Some(temporal));
        }
        Ok(DpbParameters {
            sets,
            end_bit: b.position(),
        })
    }
}
impl OutputLayerSets {
    /// Read representation formats and POC flags, stopping immediately before
    /// dpb_size(). SPS-dependent MinCbSizeY alignment still requires admission
    /// against the activating SPS; this metadata prefix cannot establish it.
    pub fn representation_formats(
        &self,
        bytes: &[u8],
        graph: &LayerDependencyPrefix,
        layers: &super::hevc_vps::LayerSets,
        budget: usize,
    ) -> Result<RepresentationFormats> {
        let n = graph.layer_ids.len();
        if n == 0 || n > 63 || graph.direct.len() != n {
            return Err(invalid("invalid HEVC representation layer geometry"));
        }
        let mut b = BitReader::new(bytes);
        b.skip(self.end_bit)?;
        let last = bounded(&mut b, 255)? as usize;
        if (last + 1) * std::mem::size_of::<RepresentationFormat>()
            + n * 2
            + std::mem::size_of::<RepresentationFormats>()
            > budget
        {
            return Err(invalid(
                "HEVC representation metadata exceeds memory budget",
            ));
        }
        let mut formats: Vec<RepresentationFormat> = Vec::with_capacity(last + 1);
        for i in 0..=last {
            let width = b.read(16)? as u16;
            let height = b.read(16)? as u16;
            if width == 0 || height == 0 {
                return Err(invalid("zero HEVC VPS representation dimensions"));
            }
            let (chroma, separate, luma_depth, chroma_depth) = if b.bit()? {
                let chroma = b.read(2)? as u8;
                let separate = chroma == 3 && b.bit()?;
                let luma = b.read(4)? as u8 + 8;
                let colour = b.read(4)? as u8 + 8;
                if luma > 16 || colour > 16 {
                    return Err(invalid("HEVC VPS representation depth exceeds 16 bits"));
                }
                (chroma, separate, luma, colour)
            } else {
                let p = formats.last().ok_or_else(|| {
                    invalid("first HEVC VPS representation format lacks chroma/depth")
                })?;
                (
                    p.chroma,
                    p.separate_colour_planes,
                    p.luma_depth,
                    p.chroma_depth,
                )
            };
            let mut crop = [0u32; 4];
            if b.bit()? {
                for value in &mut crop {
                    *value = b.unsigned_golomb()?;
                }
            }
            let format = RepresentationFormat {
                width,
                height,
                chroma,
                separate_colour_planes: separate,
                luma_depth,
                chroma_depth,
                crop,
            };
            format.display_size()?;
            formats.push(format);
            debug_assert!(i <= 255);
        }
        let present = last > 0 && b.bit()?;
        let mut indices: Vec<u8> = (0..n).map(|i| i.min(last) as u8).collect();
        if present {
            let width = (usize::BITS - last.leading_zeros()) as u8;
            for index in indices.iter_mut().skip(usize::from(layers.base_internal)) {
                *index = b.read(width)? as u8;
                if usize::from(*index) > last {
                    return Err(invalid("HEVC representation format index out of range"));
                }
            }
        }
        let one = b.bit()?;
        let aligned = b.bit()?;
        let mut absent = vec![false; n];
        for (i, value) in absent.iter_mut().enumerate().skip(1) {
            if graph.direct[i] == 0 {
                *value = b.bit()?;
            }
        }
        Ok(RepresentationFormats {
            formats,
            layer_format_indices: indices,
            max_one_active_reference: one,
            poc_lsb_aligned: aligned,
            poc_lsb_not_present: absent,
            end_bit: b.position(),
        })
    }
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
        let base_additional_profile = if layers.max_layers_minus1 > 0 && layers.base_internal {
            Some(ProfileTierLevel::read(&mut b, false, sublayers)?)
        } else {
            None
        };
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
            base_additional_profile,
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

#[cfg(test)]
mod texture_view_index_tests {
    use super::*;
    #[test]
    fn view_indices_follow_scalability_dimensions_even_with_unrelated_or_equal_view_ids() {
        let mut dimensions = vec![[0;16];4];
        dimensions[1][0]=1;
        dimensions[2][1]=2;
        dimensions[3][0]=1; dimensions[3][1]=2;
        let mut graph = LayerDependencyPrefix {base_additional_profile:None,
            layer_ids:vec![0,3,5,8],scalability_ids:dimensions,view_ids:vec![900,17],
            direct:vec![0;4],transitive:vec![0;4],tree_partitions:vec![],end_bit:0};
        for ids in [vec![900,17],vec![7,7]] {
            graph.view_ids=ids;
            for (layer,view) in [(0,0),(3,0),(5,2),(8,2)] {
                assert_eq!(graph.view_index(layer).unwrap(),view);
            }
        }
        assert!(graph.view_index(2).is_err());
        graph.scalability_ids.pop();
        assert!(graph.view_index(0).is_err());
    }
}

#[cfg(test)]
mod inter_view_identifier_tests {
    use super::*;
    #[test]
    fn view_values_use_indices_preserve_equal_ids_and_refuse_missing_entries() {
        let mut dims=vec![[0;16];3];dims[1][1]=1;dims[2][0]=1;dims[2][1]=1;
        let mut graph=LayerDependencyPrefix {base_additional_profile:None,
            layer_ids:vec![0,3,5],scalability_ids:dims,view_ids:vec![900,17],
            direct:vec![0;3],transitive:vec![0;3],tree_partitions:vec![],end_bit:0};
        for (layer,wanted)in [(0,900),(3,17),(5,17)] {
            assert_eq!(graph.view_id_value(layer).unwrap(),wanted);
        }
        graph.view_ids=vec![7,7];
        assert_eq!(graph.view_id_value(0).unwrap(),graph.view_id_value(5).unwrap());
        graph.view_ids[1]=32768;assert!(graph.view_id_value(3).is_err());
        graph.view_ids.pop();assert!(graph.view_id_value(3).is_err());
        assert!(graph.view_id_value(2).is_err());
    }
}
