//! Context and derived Annex F SPS fields shared with the ordinary SPS reader.
use super::super::{
    bits::BitReader,
    hevc_scaling::ScalingLists,
    hevc_vps::{LayeredVps, Ordering, RepresentationFormat},
};
use super::{Sps, ue};
use crate::{Result, invalid};

/// Active parameter sets for the selected layer and output layer set. References
/// are already parsed SPSs, keyed by their active layer, not by their NAL layer.
pub struct LayeredSpsContext<'a> {
    pub vps: &'a LayeredVps,
    pub layer_id: u8,
    pub output_layer_set: usize,
    pub active_sps: &'a [LayeredSps],
}
/// Resolved parameter syntax, not admission of inter-layer picture decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayeredSps {
    pub sps: Sps,
    /// VPS-selected profile for dependent layers; their shared base SPS PTL
    /// continues to describe the base layer rather than the dependent layer.
    pub effective_profile: super::super::hevc_profile::ProfileTierLevel,
    pub nal_layer_id: u8,
    pub layer_id: u8,
    pub output_layer_set: usize,
    pub multilayer_syntax: bool,
    /// SPS syntax bound, which may exceed the selected OLS's temporal depth.
    pub max_sub_layers_minus1: u8,
    pub representation_index: Option<u8>,
    pub scaling_list_reference: Option<u8>,
    /// Effective Annex F output/storage limits. OLS zero uses the base SPS.
    pub output_ordering: Vec<Ordering>,
}
impl LayeredSps {
    pub fn parse(nal: &[u8], context: &LayeredSpsContext<'_>, budget: usize) -> Result<Self> {
        Sps::parse_internal(nal, budget, Some(context))
    }
}
impl LayeredSpsContext<'_> {
    fn index(&self) -> Result<usize> {
        if self.layer_id == 0 && self.vps.extension.is_none() && self.vps.vps.layers.base_internal {
            return Ok(0);
        }
        self.vps
            .extension
            .as_ref()
            .and_then(|e| e.graph.layer_ids.iter().position(|&id| id == self.layer_id))
            .ok_or_else(|| invalid("HEVC SPS active layer is absent from VPS"))
    }
    pub(super) fn dependent(&self) -> Result<bool> {
        let i = self.index()?;
        Ok(self
            .vps
            .extension
            .as_ref()
            .and_then(|e| e.graph.direct.get(i))
            .copied()
            .unwrap_or(0)
            != 0)
    }
    pub(super) fn validate_nal_layer(&self, nal_layer: u8) -> Result<()> {
        let i = self.index()?;
        let reference = self
            .vps
            .extension
            .as_ref()
            .and_then(|e| e.graph.transitive.get(i))
            .copied()
            .unwrap_or(0);
        if nal_layer == self.layer_id
            || (nal_layer == 0 && self.vps.vps.layers.base_internal)
            || (nal_layer < 64 && reference & (1u64 << nal_layer) != 0)
        {
            Ok(())
        } else {
            Err(invalid(
                "HEVC SPS NAL does not belong to the active or a reference layer",
            ))
        }
    }
    pub(super) fn format(&self, update: Option<u8>) -> Result<(u8, &RepresentationFormat)> {
        let i = self.index()?;
        let formats = &self
            .vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("HEVC SPS has no VPS representations"))?
            .representations;
        if update.is_some() && formats.formats.len() == 1 {
            return Err(invalid(
                "HEVC SPS cannot update a single VPS representation",
            ));
        }
        let index = update
            .or_else(|| formats.layer_format_indices.get(i).copied())
            .ok_or_else(|| invalid("HEVC SPS layer has no VPS representation mapping"))?;
        let format = formats
            .formats
            .get(usize::from(index))
            .ok_or_else(|| invalid("HEVC SPS representation index is outside VPS"))?;
        Ok((index, format))
    }
    pub(super) fn ordering(&self) -> Result<Option<Vec<Ordering>>> {
        let Some(ext) = self.vps.extension.as_ref() else {
            return Ok(None);
        };
        let set = ext
            .output
            .sets
            .get(self.output_layer_set)
            .ok_or_else(|| invalid("HEVC SPS output layer set is absent"))?;
        let layers = ext
            .layer_sets
            .layer_sets
            .get(set.layer_set_index)
            .ok_or_else(|| invalid("HEVC SPS output layer list is absent"))?;
        let i = layers
            .iter()
            .position(|&id| id == self.layer_id)
            .ok_or_else(|| invalid("HEVC SPS active layer is absent from output set"))?;
        if self.output_layer_set == 0 {
            return Ok(None);
        }
        let rows = ext
            .dpb
            .sets
            .get(self.output_layer_set)
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid("HEVC SPS output DPB parameters are absent"))?;
        if rows.is_empty() || rows.len() > self.vps.vps.ordering.len() || rows.len() > 7 {
            return Err(invalid("invalid HEVC SPS output DPB temporal count"));
        }
        rows.iter()
            .map(|row| {
                let count = row
                    .decoded_pictures
                    .get(i)
                    .copied()
                    .flatten()
                    .ok_or_else(|| invalid("HEVC SPS active layer is unnecessary for output"))?;
                if !(1..=16).contains(&count) {
                    return Err(invalid("HEVC SPS per-layer DPB exceeds sixteen pictures"));
                }
                Ok(Ordering {
                    max_decoded_pictures: count as u8,
                    max_reorder_pictures: row.reorder_pictures,
                    max_latency_increase_plus1: row.latency_increase_plus1,
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }
    pub(super) fn scaling_lists(&self, reference: u8) -> Result<&ScalingLists> {
        let i = self.index()?;
        let refs = self
            .vps
            .extension
            .as_ref()
            .and_then(|e| e.graph.transitive.get(i))
            .copied()
            .unwrap_or(0);
        if reference > 62
            || (!self.vps.vps.layers.base_internal && reference == 0)
            || refs & (1u64 << reference) == 0
        {
            return Err(invalid(
                "HEVC SPS scaling-list layer is not an available reference",
            ));
        }
        let mut matches = self.active_sps.iter().filter(|s| s.layer_id == reference);
        let active = matches
            .next()
            .ok_or_else(|| invalid("HEVC SPS scaling-list reference is not active"))?;
        if matches.next().is_some()
            || active.sps.vps_id != self.vps.vps.id
            || active.scaling_list_reference.is_some()
            || !active.sps.scaling_lists_enabled
        {
            return Err(invalid(
                "invalid HEVC SPS scaling-list reference parameters",
            ));
        }
        Ok(&active.sps.scaling_lists)
    }
}
pub(super) struct PictureFormat {
    pub chroma: u8,
    pub separate: bool,
    pub dimensions: [u32; 2],
    pub crop: [u32; 4],
    pub depth: [u8; 2],
}
impl PictureFormat {
    fn crop(b: &mut BitReader<'_>, chroma: u8, separate: bool) -> Result<[u32; 4]> {
        let coded = if b.bit()? {
            [
                b.unsigned_golomb()?,
                b.unsigned_golomb()?,
                b.unsigned_golomb()?,
                b.unsigned_golomb()?,
            ]
        } else {
            [0; 4]
        };
        Self::scale_crop(coded, chroma, separate)
    }
    fn scale_crop(coded: [u32; 4], chroma: u8, separate: bool) -> Result<[u32; 4]> {
        let scale = if separate {
            [1, 1]
        } else {
            match chroma {
                1 => [2, 2],
                2 => [2, 1],
                _ => [1, 1],
            }
        };
        let mut crop = [0; 4];
        for i in 0..4 {
            crop[i] = coded[i]
                .checked_mul(scale[i / 2])
                .ok_or_else(|| invalid("HEVC conformance crop overflow"))?;
        }
        Ok(crop)
    }
    fn validate(self) -> Result<Self> {
        if self.dimensions.contains(&0) {
            return Err(invalid("zero HEVC picture dimension"));
        }
        for axis in 0..2 {
            if u64::from(self.crop[axis * 2]) + u64::from(self.crop[axis * 2 + 1])
                >= u64::from(self.dimensions[axis])
            {
                return Err(invalid("HEVC conformance crop removes picture"));
            }
        }
        Ok(self)
    }
    pub(super) fn read(b: &mut BitReader<'_>) -> Result<Self> {
        let chroma = ue(b, 3)? as u8;
        let separate = chroma == 3 && b.bit()?;
        let dimensions = [b.unsigned_golomb()?, b.unsigned_golomb()?];
        if dimensions.contains(&0) {
            return Err(invalid("zero HEVC picture dimension"));
        }
        let crop = Self::crop(b, chroma, separate)?;
        let depth = [ue(b, 8)? as u8 + 8, ue(b, 8)? as u8 + 8];
        Self {
            chroma,
            separate,
            dimensions,
            crop,
            depth,
        }
        .validate()
    }
    pub(super) fn from_vps(f: &RepresentationFormat) -> Result<Self> {
        Self {
            chroma: f.chroma,
            separate: f.separate_colour_planes,
            dimensions: [u32::from(f.width), u32::from(f.height)],
            crop: Self::scale_crop(f.crop, f.chroma, f.separate_colour_planes)?,
            depth: [f.luma_depth, f.chroma_depth],
        }
        .validate()
    }
    pub(super) fn within(&self, maximum: &RepresentationFormat) -> Result<()> {
        if self.chroma > maximum.chroma
            || self.dimensions[0] > u32::from(maximum.width)
            || self.dimensions[1] > u32::from(maximum.height)
            || self.depth[0] > maximum.luma_depth
            || self.depth[1] > maximum.chroma_depth
        {
            return Err(invalid(
                "coded HEVC SPS format exceeds its VPS representation",
            ));
        }
        Ok(())
    }
}
