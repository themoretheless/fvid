//! Active-layer PPS syntax, reference regions and scaling-list inference.
use super::super::{
    bits::BitReader,
    hevc_scaling::ScalingLists,
    hevc_sps::{LayeredSps, LayeredSpsContext, Sps},
    hevc_vps::LayeredVps,
};
use super::{ColourMappingTable, Pps, se, ue};
use crate::{Result, invalid};
pub struct LayeredPpsContext<'a> {
    pub vps: &'a LayeredVps,
    pub sps: &'a LayeredSps,
    pub active_sps: &'a [LayeredSps],
    pub active_pps: &'a [LayeredPps],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceLocation {
    pub layer_id: u8,
    /// Left/top/right/bottom, in current/reference chroma subsampling units.
    pub scaled_offsets: Option<[i32; 4]>,
    pub region_offsets: Option<[i32; 4]>,
    /// Raw luma phases and chroma phases plus eight. None preserves inference.
    /// Non-direct-reference phase values are unspecified and must be ignored.
    pub phases: Option<[u32; 4]>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceGeometry {
    pub scaled_offsets: [i32; 4],
    pub reference_offsets: [i32; 4],
    pub scaled_size: [u64; 2],
    pub reference_size: [u64; 2],
    /// Signed effective phases; chroma no longer has the plus-eight bias.
    pub phases: [i64; 4],
}
fn subsampling(s: &Sps) -> [i32; 2] {
    if s.separate_colour_plane {
        [1, 1]
    } else {
        match s.chroma_format {
            1 => [2, 2],
            2 => [2, 1],
            _ => [1, 1],
        }
    }
}
impl ReferenceLocation {
    pub fn geometry(
        &self,
        current: &Sps,
        reference: &Sps,
        direct: bool,
    ) -> Result<ReferenceGeometry> {
        fn offsets(value: Option<[i32; 4]>, scale: [i32; 2]) -> Result<[i32; 4]> {
            let mut result = value.unwrap_or([0; 4]);
            for i in 0..4 {
                result[i] = result[i]
                    .checked_mul(scale[i % 2])
                    .ok_or_else(|| invalid("HEVC reference offset overflow"))?;
            }
            Ok(result)
        }
        fn size(d: [u32; 2], o: [i32; 4]) -> Result<[u64; 2]> {
            let w = i64::from(d[0]) - i64::from(o[0]) - i64::from(o[2]);
            let h = i64::from(d[1]) - i64::from(o[1]) - i64::from(o[3]);
            if w <= 0 || h <= 0 {
                return Err(invalid("HEVC reference offsets remove the picture region"));
            }
            Ok([w as u64, h as u64])
        }
        let scaled_offsets = offsets(self.scaled_offsets, subsampling(current))?;
        let reference_offsets = offsets(self.region_offsets, subsampling(reference))?;
        let scaled_size = size(current.dimensions, scaled_offsets)?;
        let reference_size = size(reference.dimensions, reference_offsets)?;
        let vertical = if matches!(current.chroma_format, 0 | 3) {
            8
        } else {
            (4 * scaled_size[1] + reference_size[1] / 2) / reference_size[1] + 4
        };
        let phases = if direct {
            self.phases.map(|p| p.map(i64::from))
        } else {
            None
        }
        .unwrap_or([0, 0, 8, vertical as i64]);
        if direct && (phases[0] > 31 || phases[1] > 31 || phases[2] > 63 || phases[3] > 63) {
            return Err(invalid(
                "HEVC direct-reference resampling phase exceeds range",
            ));
        }
        Ok(ReferenceGeometry {
            scaled_offsets,
            reference_offsets,
            scaled_size,
            reference_size,
            phases: if current.chroma_format == 0 { [phases[0], phases[1], 0, 0] }
                else { [phases[0], phases[1], phases[2] - 8, phases[3] - 8] },
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayeredPps {
    pub pps: Pps,
    pub vps_id: u8,
    pub layer_id: u8,
    pub nal_layer_id: u8,
    pub output_layer_set: usize,
    pub scaling_list_data_present: bool,
    pub scaling_list_reference: Option<u8>,
    pub effective_scaling_lists: ScalingLists,
    pub reference_locations: Vec<ReferenceLocation>,
    pub colour_mapping: Option<ColourMappingTable>,
}
impl LayeredPps {
    fn reference_source<'a>(
        &self,
        context: &'a LayeredPpsContext<'_>,
        reference_layer: u8,
        motion: bool,
    ) -> Result<&'a LayeredSps> {
        if self.layer_id != context.sps.layer_id
            || self.output_layer_set != context.sps.output_layer_set
            || self.vps_id != context.vps.vps.id
            || context.sps.sps.vps_id != context.vps.vps.id
            || self.pps.sps_id != context.sps.sps.id
        {
            return Err(invalid(
                "HEVC inter-layer reference uses mismatched active parameters",
            ));
        }
        context
            .vps
            .profile_for_layer(self.output_layer_set, self.layer_id)?;
        if reference_layer > 62 || context.direct()? & (1u64 << reference_layer) == 0 {
            return Err(invalid("HEVC source is not a direct reference layer"));
        }
        let extension = context
            .vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("HEVC source has no VPS extension"))?;
        let index = extension
            .graph
            .layer_ids
            .iter()
            .position(|&id| id == self.layer_id)
            .ok_or_else(|| invalid("HEVC sample layer is absent from VPS"))?;
        let enabled = if motion {
            &extension.dependencies.motion_prediction
        } else {
            &extension.dependencies.sample_prediction
        };
        if enabled.get(index).copied().unwrap_or(0) & (1u64 << reference_layer) == 0 {
            return Err(invalid(if motion {
                "HEVC VPS disables inter-layer motion prediction"
            } else {
                "HEVC VPS disables inter-layer sample prediction"
            }));
        }
        context.active_sps(reference_layer)
    }
    /// Prepare H.8.1.4.3 motion/mode metadata from a direct reference layer.
    /// Sample-only dependencies cannot be requested as motion sources.
    pub fn reference_motion(
        &self,
        context: &LayeredPpsContext<'_>,
        reference_layer: u8,
        input: super::super::hevc_resample::MotionSource<'_>,
        budget: usize,
    ) -> Result<Vec<super::super::hevc_motion::Motion>> {
        let reference = self.reference_source(context, reference_layer, true)?;
        let default = ReferenceLocation {
            layer_id: reference_layer,
            scaled_offsets: None,
            region_offsets: None,
            phases: None,
        };
        let location = self
            .reference_locations
            .iter()
            .rfind(|loc| loc.layer_id == reference_layer)
            .unwrap_or(&default);
        super::super::hevc_resample::MotionResampling::new(
            &context.sps.sps,
            &reference.sps,
            location,
        )?
        .motion(input, budget)
    }
    /// H.8.1.4.1 reference samples: colour mapping precedes spatial resampling.
    /// This does not derive motion/mode arrays or attach the result to a DPB.
    /// Budget covers all new sample buffers alive simultaneously, including
    /// mapped intermediate planes. Input and active parameter storage are borrowed.
    pub fn reference_samples(
        &self,
        context: &LayeredPpsContext<'_>,
        reference_layer: u8,
        input: [&[u16]; 3],
        budget: usize,
    ) -> Result<[super::super::hevc_plane::Plane; 3]> {
        use super::super::hevc_resample::Resampling;
        use super::ColourMappingPicture;
        let reference = self.reference_source(context, reference_layer, false)?;
        let default = ReferenceLocation {
            layer_id: reference_layer,
            scaled_offsets: None,
            region_offsets: None,
            phases: None,
        };
        let location = self
            .reference_locations
            .iter()
            .rfind(|loc| loc.layer_id == reference_layer)
            .unwrap_or(&default);
        let mapping = self
            .colour_mapping
            .as_ref()
            .filter(|table| table.reference_layers.contains(&reference_layer));
        let depth = mapping.map_or(reference.sps.depth, |table| table.output_depth);
        let resampling =
            Resampling::with_reference_depth(&context.sps.sps, &reference.sps, depth, location)?;
        if let Some(table) = mapping {
            if table.input_depth != reference.sps.depth {
                return Err(invalid(
                    "HEVC colour mapping input depth contradicts active reference",
                ));
            }
            let mapped = table.map_picture(
                ColourMappingPicture {
                    dimensions: reference.sps.dimensions.map(|v| v as usize),
                    chroma_format: reference.sps.chroma_format,
                    depth: reference.sps.depth,
                    planes: input,
                },
                budget,
            )?;
            if resampling.is_identity() {
                return Ok(mapped);
            }
            let mut held = std::mem::size_of_val(&mapped);
            for plane in &mapped {
                held = held
                    .checked_add(
                        plane
                            .storage_bytes()
                            .ok_or_else(|| invalid("HEVC mapped reference storage overflow"))?,
                    )
                    .ok_or_else(|| invalid("HEVC mapped reference storage overflow"))?;
            }
            let remaining = budget
                .checked_sub(held)
                .ok_or_else(|| invalid("HEVC mapped reference exceeds memory budget"))?;
            resampling.picture(std::array::from_fn(|c| mapped[c].samples()), remaining)
        } else {
            resampling.picture(input, budget)
        }
    }
    /// Attach H.8.1.4 samples and motion to an immutable inter-layer reference.
    /// Source picture/parameters are borrowed; budget covers all additional live
    /// allocations. The output is a prediction reference, not a display picture.
    pub fn reference_picture(
        &self,
        context: &LayeredPpsContext<'_>,
        reference_layer: u8,
        source: &super::super::hevc_picture::Picture,
        budget: usize,
    ) -> Result<super::super::hevc_picture::Picture> {
        use super::super::hevc_motion::Motion;
        use std::mem::size_of;
        if reference_layer > 62 {
            return Err(invalid("HEVC source is not a direct reference layer"));
        }
        let extension = context
            .vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("HEVC source has no VPS extension"))?;
        let index = extension
            .graph
            .layer_ids
            .iter()
            .position(|&id| id == self.layer_id)
            .ok_or_else(|| invalid("HEVC sample layer is absent from VPS"))?;
        let sample_enabled = extension
            .dependencies
            .sample_prediction
            .get(index)
            .copied()
            .unwrap_or(0)
            & (1u64 << reference_layer)
            != 0;
        let reference = self.reference_source(context, reference_layer, !sample_enabled)?;
        if source.dimensions != reference.sps.dimensions || source.depth != reference.sps.depth {
            return Err(invalid(
                "HEVC inter-layer source picture disagrees with active SPS",
            ));
        }
        let metadata = source
            .poc_translation_bytes()
            .ok_or_else(|| invalid("HEVC prepared reference metadata overflow"))?;
        let default = ReferenceLocation {
            layer_id: reference_layer,
            scaled_offsets: None,
            region_offsets: None,
            phases: None,
        };
        let location = self
            .reference_locations
            .iter()
            .rfind(|l| l.layer_id == reference_layer)
            .unwrap_or(&default);
        let geometry = location.geometry(&context.sps.sps, &reference.sps, true)?;
        let mapping = self
            .colour_mapping
            .as_ref()
            .is_some_and(|t| t.reference_layers.contains(&reference_layer));
        // H.8.1.4.1 shares the decoded reference, including motion, for equal
        // representations. Do not discard motion merely because its separate
        // resampling dependency is disabled in this no-processing case.
        if context.sps.sps.dimensions == reference.sps.dimensions
            && context.sps.sps.depth == reference.sps.depth
            && context.sps.sps.chroma_format == reference.sps.chroma_format
            && context.sps.sps.separate_colour_plane == reference.sps.separate_colour_plane
            && geometry.scaled_offsets == geometry.reference_offsets
            && geometry.phases == [0; 4]
            && !mapping
        {
            return source.translated_layer_pocs(&[0; 63], budget);
        }
        let available = budget
            .checked_sub(metadata)
            .ok_or_else(|| invalid("HEVC prepared reference exceeds memory budget"))?;
        let planes = self.reference_samples(
            context,
            reference_layer,
            std::array::from_fn(|c| source.planes[c].samples()),
            available,
        )?;
        let held = planes
            .iter()
            .try_fold(metadata, |n, p| n.checked_add(p.storage_bytes()?))
            .ok_or_else(|| invalid("HEVC prepared reference sample storage overflow"))?;
        let remaining = budget
            .checked_sub(held)
            .ok_or_else(|| invalid("HEVC prepared reference exceeds memory budget"))?;
        let motion_enabled = extension
            .dependencies
            .motion_prediction
            .get(index)
            .copied()
            .unwrap_or(0)
            & (1u64 << reference_layer)
            != 0;
        let motion = if motion_enabled {
            self.reference_motion(
                context,
                reference_layer,
                // Owned Picture stores an empty motion map only for an all-I
                // reconstruction; inter pictures always retain a complete grid.
                if source.motion.is_empty() {
                    super::super::hevc_resample::MotionSource::Intra
                } else {
                    super::super::hevc_resample::MotionSource::Grid(&source.motion)
                },
                remaining,
            )?
        } else {
            let [w, h] = context.sps.sps.dimensions;
            let count = (w as usize)
                .div_ceil(16)
                .checked_mul((h as usize).div_ceil(16))
                .ok_or_else(|| invalid("HEVC prepared reference motion size overflow"))?;
            if count
                .checked_mul(size_of::<Motion>())
                .and_then(|n| n.checked_add(size_of::<Vec<Motion>>()))
                .is_none_or(|n| n > remaining)
            {
                return Err(invalid(
                    "HEVC prepared reference motion exceeds memory budget",
                ));
            }
            let mut grid = Vec::new();
            grid.try_reserve_exact(count)
                .map_err(|_| invalid("cannot allocate HEVC prepared reference motion"))?;
            grid.resize(count, [None; 2]);
            grid
        };
        // Existing bounded copier preserves full layer/POC/reference-class maps,
        // including their index correspondence with the resampled motion grid.
        let mut prepared = source.translated_layer_pocs(&[0; 63], metadata)?;
        prepared.dimensions = context.sps.sps.dimensions;
        prepared.crop = context.sps.sps.crop;
        prepared.depth = context.sps.sps.depth;
        prepared.planes = planes;
        // Processed sample representations have no newly coded CU geometry.
        // Keep original maps only in the exact identity-sharing path above.
        prepared.coding = None;
        prepared.motion = motion;
        prepared.sao.clear();
        if !motion_enabled {
            prepared.reference_pocs = [Vec::new(), Vec::new()];
            prepared.reference_layers = [Vec::new(), Vec::new()];
            prepared.reference_long_term = [Vec::new(), Vec::new()];
        }
        Ok(prepared)
    }
    pub fn parse(nal: &[u8], context: &LayeredPpsContext<'_>, budget: usize) -> Result<Self> {
        if context.sps.sps.vps_id != context.vps.vps.id {
            return Err(invalid("HEVC PPS SPS belongs to another VPS"));
        }
        context
            .vps
            .profile_for_layer(context.sps.output_layer_set, context.sps.layer_id)?;
        Pps::parse_internal(nal, &context.sps.sps, budget, Some(context))
    }
}
pub(super) struct Extension {
    pub poc_reset: bool,
    pub scaling_reference: Option<u8>,
    pub locations: Vec<ReferenceLocation>,
    pub colour_mapping: Option<ColourMappingTable>,
}
impl LayeredPpsContext<'_> {
    pub(super) fn validate_nal_layer(&self, layer: u8) -> Result<()> {
        LayeredSpsContext {
            vps: self.vps,
            layer_id: self.sps.layer_id,
            output_layer_set: self.sps.output_layer_set,
            active_sps: self.active_sps,
        }
        .validate_nal_layer(layer)
    }
    fn direct(&self) -> Result<u64> {
        let Some(ext) = self.vps.extension.as_ref() else {
            return Ok(0);
        };
        let i = ext
            .graph
            .layer_ids
            .iter()
            .position(|&id| id == self.sps.layer_id)
            .ok_or_else(|| invalid("HEVC PPS active layer is absent from VPS"))?;
        ext.graph
            .direct
            .get(i)
            .copied()
            .ok_or_else(|| invalid("HEVC PPS layer dependencies are absent"))
    }
    fn active_sps(&self, id: u8) -> Result<&LayeredSps> {
        let mut values = self.active_sps.iter().filter(|s| s.layer_id == id);
        let value = values
            .next()
            .ok_or_else(|| invalid("HEVC PPS reference SPS is not active"))?;
        if values.next().is_some() || value.sps.vps_id != self.vps.vps.id {
            return Err(invalid("ambiguous or foreign HEVC PPS reference SPS"));
        }
        Ok(value)
    }
    pub(super) fn scaling_lists(&self, id: u8) -> Result<&ScalingLists> {
        let ext = self
            .vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("HEVC PPS scaling reference has no VPS graph"))?;
        let i = ext
            .graph
            .layer_ids
            .iter()
            .position(|&layer| layer == self.sps.layer_id)
            .ok_or_else(|| invalid("HEVC PPS active layer is absent"))?;
        let refs = ext
            .graph
            .transitive
            .get(i)
            .copied()
            .ok_or_else(|| invalid("HEVC PPS reference closure is absent"))?;
        if id > 62 || (!self.vps.vps.layers.base_internal && id == 0) || refs & (1u64 << id) == 0 {
            return Err(invalid(
                "HEVC PPS scaling-list layer is not an available reference",
            ));
        }
        if !self.active_sps(id)?.sps.scaling_lists_enabled {
            return Err(invalid(
                "HEVC PPS scaling-list reference SPS disables scaling",
            ));
        }
        let mut values = self.active_pps.iter().filter(|p| p.layer_id == id);
        let p = values
            .next()
            .ok_or_else(|| invalid("HEVC PPS scaling-list reference PPS is not active"))?;
        if values.next().is_some()
            || p.scaling_list_reference.is_some()
            || p.vps_id != self.vps.vps.id
        {
            return Err(invalid(
                "ambiguous or chained HEVC PPS scaling-list reference",
            ));
        }
        if p.pps.sps_id != self.active_sps(id)?.sps.id {
            return Err(invalid("HEVC PPS scaling reference uses another SPS"));
        }
        Ok(&p.effective_scaling_lists)
    }
    pub(super) fn read_extension(
        &self,
        b: &mut BitReader<'_>,
        id: u8,
        coded_scaling: bool,
        budget: usize,
    ) -> Result<Extension> {
        let poc_reset = b.bit()?;
        let scaling_reference = if b.bit()? {
            if !self.sps.sps.scaling_lists_enabled || self.sps.layer_id == 0 || coded_scaling {
                return Err(invalid(
                    "HEVC PPS scaling-list inference contradicts active parameters",
                ));
            }
            Some(b.read(6)? as u8)
        } else {
            None
        };
        if let Some(id) = scaling_reference {
            self.scaling_lists(id)?;
        }
        let count = ue(b, u32::from(self.vps.vps.layers.max_layers_minus1))? as usize;
        let used = count * std::mem::size_of::<ReferenceLocation>();
        if used > budget {
            return Err(invalid("HEVC PPS reference metadata exceeds memory budget"));
        }
        let direct = self.direct()?;
        let mut locations = Vec::with_capacity(count);
        for _ in 0..count {
            let layer = b.read(6)? as u8;
            let read_offsets = |b: &mut BitReader<'_>| -> Result<Option<[i32; 4]>> {
                if b.bit()? {
                    Ok(Some([
                        se(b, -16384, 16383)?,
                        se(b, -16384, 16383)?,
                        se(b, -16384, 16383)?,
                        se(b, -16384, 16383)?,
                    ]))
                } else {
                    Ok(None)
                }
            };
            let scaled_offsets = read_offsets(b)?;
            let region_offsets = read_offsets(b)?;
            let phases = if b.bit()? {
                let values = [
                    b.unsigned_golomb()?,
                    b.unsigned_golomb()?,
                    b.unsigned_golomb()?,
                    b.unsigned_golomb()?,
                ];
                if direct & (1u64 << layer) != 0
                    && (values[0] > 31 || values[1] > 31 || values[2] > 63 || values[3] > 63)
                {
                    return Err(invalid(
                        "HEVC PPS direct-reference resampling phase exceeds range",
                    ));
                }
                Some(values)
            } else {
                None
            };
            locations.push(ReferenceLocation {
                layer_id: layer,
                scaled_offsets,
                region_offsets,
                phases,
            });
            if direct & (1u64 << layer) != 0 {
                locations.last().unwrap().geometry(
                    &self.sps.sps,
                    &self.active_sps(layer)?.sps,
                    true,
                )?;
            }
        }
        let colour_mapping = if b.bit()? {
            if id >= 8 {
                return Err(invalid(
                    "HEVC PPS colour mapping requires PPS ID below eight",
                ));
            }
            Some(ColourMappingTable::read(
                b,
                self.sps.sps.depth,
                direct,
                |id| Ok(self.active_sps(id)?.sps.depth),
                budget - used,
            )?)
        } else {
            None
        };
        Ok(Extension {
            poc_reset,
            scaling_reference,
            locations,
            colour_mapping,
        })
    }
}

#[cfg(test)]
mod prepared_reference_tests {
    use super::*;
    use crate::codec::{
        config::HevcConfig,
        hevc_decoder::{HevcDecoder, LayeredAccessUnit, LayeredParameters},
        hevc_poc::LayeredPocState,
    };
    #[test]
    fn identity_reference_retains_real_inter_motion_and_full_reference_identities() {
        let data =
            include_bytes!("../../tests/fixtures/playback-errors/hevc-pcm-reference-rext8.mp4");
        let mut movie =
            crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default())
                .unwrap();
        let mut decoder =
            HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20).unwrap();
        let mut packet = vec![];
        movie.read_packet(0, 0, &mut packet).unwrap();
        decoder.decode_packet(&packet).unwrap();
        movie.read_packet(0, 1, &mut packet).unwrap();
        let source = decoder.decode_packet(&packet).unwrap().unwrap().picture;
        assert!(source.motion.iter().flatten().any(Option::is_some));
        let layered = include_bytes!(
            "../../tests/fixtures/playback-errors/hevc-layered-reconstruct-inter-prediction.mp4"
        );
        let mut movie = crate::container::mp4::Mp4Reader::open(
            std::io::Cursor::new(layered),
            Default::default(),
        )
        .unwrap();
        let length_size = HevcConfig::parse(&movie.tracks()[0].configuration).unwrap().length_size;
        let parameters =
            LayeredParameters::from_configuration(&movie.tracks()[0].configuration, 32 << 20)
                .unwrap();
        movie.read_packet(0, 0, &mut packet).unwrap();
        let plan = LayeredAccessUnit::parse(
            &parameters,
            &LayeredPocState::default(),
            &packet,
            length_size,
            1,
            32 << 20,
        )
        .unwrap();
        let current = &plan.pictures()[1].binding;
        let sps: Vec<_> = plan
            .pictures()
            .iter()
            .map(|p| p.binding.sps.clone())
            .collect();
        for disable_motion in [false, true] {
            let mut vps = (*current.vps).clone();
            if disable_motion {
                vps.extension
                    .as_mut()
                    .unwrap()
                    .dependencies
                    .motion_prediction[1] = 0;
            }
            let context = LayeredPpsContext {
                vps: &vps,
                sps: &current.sps,
                active_sps: &sps,
                active_pps: &[],
            };
            let prepared = current
                .pps
                .reference_picture(&context, 0, &source, 32 << 20)
                .unwrap();
            let metadata = source.poc_translation_bytes().unwrap();
            let small = current.pps.reference_picture(&context,0,&source,metadata).unwrap();
            assert!(small.additional_reference_storage(&source).unwrap() <= metadata);
            assert_eq!(small.coding_unit_at([0,0]).unwrap(),source.coding_unit_at([0,0]).unwrap());
            let mut shifted=current.pps.clone();
            shifted.reference_locations=vec![ReferenceLocation {
                layer_id:0,scaled_offsets:None,region_offsets:Some([0,0,8,0]),phases:None,
            }];
            let processed=shifted.reference_picture(&context,0,&source,32<<20).unwrap();
            assert_eq!(processed.coding_unit_at([0,0]).err().unwrap().to_string(),"HEVC picture has no coded-CU map");

            assert_eq!(prepared.motion, source.motion);
            assert_eq!(prepared.reference_pocs, source.reference_pocs);
            assert_eq!(prepared.reference_layers, source.reference_layers);
            assert_eq!(prepared.reference_long_term, source.reference_long_term);
            for (a, b) in prepared.planes.iter().zip(&source.planes) {
                assert_eq!(a.samples(), b.samples());
            }
            assert!(
                current
                    .pps
                    .reference_picture(&context, 64, &source, 32 << 20)
                    .is_err()
            );
        }
    }
}
