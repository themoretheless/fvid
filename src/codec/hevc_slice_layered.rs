//! Annex F slice syntax bound to resolved VPS/SPS/PPS parameters.
use super::super::{
    bits::BitReader,
    hevc_nal::NalHeader,
    hevc_pps::LayeredPps,
    hevc_sps::LayeredSps,
    hevc_vps::{LayeredExtensionPrefix, LayeredVps},
};
use crate::{Result, invalid};

pub struct LayeredSliceContext<'a> {
    pub vps: &'a LayeredVps,
    pub sps: &'a LayeredSps,
    pub pps: &'a LayeredPps,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceCameraPair {
    pub reference_view:u16,
    pub values:[i32;4],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceCamera {
    pub precision:u8,
    pub view_index:u16,
    pub pairs:Vec<SliceCameraPair>,
}
impl SliceCamera {
    /// Bind Annex I.8.3.2's default view to this slice's camera transform.
    /// A missing pair is an error only when a usable default view was selected.
    pub fn default_disparity_transform(&self,
        lists:[&[super::super::hevc_motion::disparity::CandidatePicture];2],
        b_slice:bool,current_poc:i32,bit_depth:u8)
        ->Result<Option<(u16,super::super::hevc_motion::disparity::CameraTransform)>> {
        let Some(view)=super::super::hevc_motion::disparity::default_reference_view(
            lists,b_slice,self.view_index,current_poc) else {return Ok(None)};
        Ok(Some((view,self.disparity_transform(view,bit_depth)?)))
    }
    pub fn disparity_transform(&self,reference_view:u16,bit_depth:u8)
        ->Result<super::super::hevc_motion::disparity::CameraTransform> {
        let mut matches=self.pairs.iter().filter(|p|p.reference_view==reference_view);
        let pair=matches.next().ok_or_else(||invalid("HEVC camera has no parameters for reference view"))?;
        if matches.next().is_some() {return Err(invalid("duplicate HEVC slice camera reference"));}
        super::super::hevc_motion::disparity::CameraTransform::new(pair.values,self.precision,bit_depth)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayeredSliceSyntax {
    pub discardable: bool,
    pub cross_layer_bla: bool,
    pub illumination_compensation:Option<[bool;2]>,
    pub camera:Option<SliceCamera>,
    /// None distinguishes absence from a coded disabled prediction flag.
    pub inter_layer_prediction: Option<bool>,
    /// None means inCmpPredAvailFlag is zero; absence infers disabled prediction.
    pub in_component_prediction: Option<bool>,
    pub coded_count_minus1: Option<u8>,
    /// RefPicLayerId, not VPS indices or inferred temporal-eligible indices.
    pub reference_layers: Vec<u8>,
    pub poc_lsb_present: bool,
    pub poc_reset: super::super::hevc_poc::ResetSyntax,
}
impl LayeredSliceSyntax {
    pub(crate) fn inferred_base(nal: NalHeader) -> Self {
        Self {
            discardable: false,
            cross_layer_bla: false,
            illumination_compensation: None,
            camera: None,
            inter_layer_prediction: None,
            in_component_prediction: None,
            coded_count_minus1: None,
            reference_layers: Vec::new(),
            poc_lsb_present: !nal.is_idr(),
            poc_reset: Default::default(),
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct Layout<'a> {
    e: &'a LayeredExtensionPrefix,
    index: usize,
    nal: NalHeader,
    reference_mask:u64,
}
impl LayeredSliceContext<'_> {
    pub(super) fn illumination_flag_present(&self)->Result<bool> {
        let annex_i=self.vps.three_d_extension.is_some()||self.sps.effective_profile.profile
            .is_some_and(|p|p.space==0&&p.compatible_with(8));
        if !annex_i||self.sps.sps.depth_layer {return Ok(false);}
        let Some(e)=self.vps.extension.as_ref()else{return Ok(false)};
        Ok(e.graph.reference_list_layers(self.sps.layer_id)?!=0)
    }
    pub(super) fn camera_parameters(&self,b:&mut BitReader<'_>,budget:usize)->Result<Option<SliceCamera>> {
        let Some(c)=self.vps.three_d_extension.as_deref()else{return Ok(None)};
        let graph=&self.vps.extension.as_ref().ok_or_else(||invalid("HEVC camera header requires VPS graph"))?.graph;
        let view_index=graph.view_index(self.sps.layer_id)?;
        let view=c.views.iter().find(|v|v.view_index==view_index)
            .ok_or_else(||invalid("HEVC camera view is missing from VPS"))?;
        if view.pairs.is_empty() {return Ok(None);}
        let required=std::mem::size_of::<SliceCamera>()+view.pairs.len()*std::mem::size_of::<SliceCameraPair>();
        if required>budget {return Err(invalid("HEVC slice camera metadata exceeds budget"));}
        let mut bits=b.clone();let mut pairs=Vec::new();
        pairs.try_reserve_exact(view.pairs.len()).map_err(|_|invalid("HEVC slice camera allocation failed"))?;
        if std::mem::size_of::<SliceCamera>()+pairs.capacity()*std::mem::size_of::<SliceCameraPair>()>budget {
            return Err(invalid("HEVC slice camera metadata exceeds budget"));
        }
        for pair in &view.pairs {
            let values=if view.in_slice_header {
                [bits.signed_golomb()?,bits.signed_golomb()?,bits.signed_golomb()?,bits.signed_golomb()?]
            }else{pair.parameters.ok_or_else(||invalid("missing static HEVC camera values"))?};
            pairs.push(SliceCameraPair {reference_view:pair.reference_view,values});
        }
        *b=bits;
        Ok(Some(SliceCamera {precision:c.precision,view_index,pairs}))
    }
    /// Annex I-17/I-18 syntax availability, distinct from actual DPB presence.
    pub fn in_component_prediction_available(&self,nal:NalHeader,syntax:&LayeredSliceSyntax)->Result<bool> {
        let Some(layout)=self.layout(nal)? else {return Ok(false)};
        let Some(tools)=&self.sps.sps.three_d else {return Ok(false)};
        let depth=layout.e.graph.scalability_ids[layout.index][0]==1;
        if depth!=self.sps.sps.depth_layer {return Err(invalid("HEVC in-component layer role disagrees with SPS"));}
        let enabled=if depth {tools.intra_contour || tools.coding_tree_partition || tools.texture_motion}
            else {tools.view_synthesis_prediction || tools.depth_based_partition || tools.depth_reference};
        if !enabled {return Ok(false);}
        let current=[nal.layer_id];
        let components=if depth {current.as_slice()}else{syntax.reference_layers.as_slice()};
        for &layer in components {
            let Some(other)=layout.e.graph.view_component_layer(layer,!depth)? else {return Ok(false)};
            let index=layout.e.graph.layer_ids.iter().position(|&id|id==other).unwrap();
            let limit=layout.e.layer_sets.max_tid_ref_plus1[index][layout.index];
            if limit>7 {return Err(invalid("HEVC in-component temporal limit exceeds range"));}
            if layout.e.graph.direct[layout.index]&(1u64<<other)==0
                || layout.e.layer_sets.sublayers_minus1[index]<nal.temporal_id
                || nal.temporal_id!=0 && limit<=nal.temporal_id {return Ok(false);}
        }
        Ok(true)
    }
    pub(crate) fn validate_poc_header(&self, header: &super::SliceHeader) -> Result<()> {
        let layout = self.layout(header.nal)?;
        if layout.is_none() {
            if !self.vps.inferred_base_layer()
                || !(4..=16).contains(&self.sps.sps.poc_bits)
                || header.pps_id != self.pps.pps.id
                || header.poc_lsb >= (1u32 << self.sps.sps.poc_bits)
                || header.layered.as_ref() != Some(&LayeredSliceSyntax::inferred_base(header.nal))
            {
                return Err(invalid("HEVC inferred base POC context disagrees"));
            }
            return Ok(());
        }
        let layout = layout.unwrap();
        let syntax = header
            .layered
            .as_ref()
            .ok_or_else(|| invalid("HEVC layered POC has no syntax"))?;
        if !(4..=16).contains(&self.sps.sps.poc_bits)
            || header.pps_id != self.pps.pps.id
            || header.poc_lsb >= (1u32 << self.sps.sps.poc_bits)
            || layout.reset(self, syntax, &header.extension, header.poc_lsb)? != syntax.poc_reset
            || syntax.in_component_prediction.is_some()!=self.in_component_prediction_available(header.nal,syntax)?
        {
            return Err(invalid("HEVC layered POC header context disagrees"));
        }
        Ok(())
    }
    pub(super) fn layout(&self, nal: NalHeader) -> Result<Option<Layout<'_>>> {
        if self.pps.layer_id != nal.layer_id
            || self.sps.layer_id != nal.layer_id
            || self.pps.output_layer_set != self.sps.output_layer_set
            || self.pps.vps_id != self.vps.vps.id
            || self.sps.sps.vps_id != self.vps.vps.id
            || self.pps.pps.sps_id != self.sps.sps.id
        {
            return Err(invalid("HEVC layered slice active parameters disagree"));
        }
        if nal.temporal_id > self.sps.max_sub_layers_minus1
            || self
                .sps
                .output_ordering
                .get(usize::from(nal.temporal_id))
                .is_none()
        {
            return Err(invalid(
                "HEVC layered slice exceeds active temporal sublayers",
            ));
        }
        let Some(e) = self.vps.extension.as_ref() else {
            if nal.layer_id != 0
                || self.sps.output_layer_set != 0
                || !self.vps.vps.layers.base_internal
            {
                return Err(invalid("HEVC layered slice has no VPS extension"));
            }
            return Ok(None);
        };
        let ids = &e.graph.layer_ids;
        let n = ids.len();
        if n == 0
            || n > 63
            || ids[0] != 0
            || ids.iter().any(|&id| id > 62)
            || ids.windows(2).any(|p| p[0] >= p[1])
            || e.graph.direct.len() != n
            || e.layer_sets.sublayers_minus1.len() != n
            || e.representations.poc_lsb_not_present.len() != n
            || e.layer_sets.max_tid_ref_plus1.len() != n
            || e.layer_sets.max_tid_ref_plus1.iter().any(|r| r.len() != n)
        {
            return Err(invalid("invalid HEVC layered slice VPS graph"));
        }
        let index = ids
            .binary_search(&nal.layer_id)
            .map_err(|_| invalid("HEVC slice layer is absent from VPS"))?;
        let lower = ids[..index]
            .iter()
            .fold(0u64, |mask, &id| mask | (1u64 << id));
        if e.graph.direct[index] & !lower != 0 {
            return Err(invalid("invalid HEVC slice direct reference layers"));
        }
        if nal.temporal_id > e.layer_sets.sublayers_minus1[index] {
            return Err(invalid("HEVC layered slice exceeds VPS temporal sublayers"));
        }
        if e.output
            .sets
            .get(self.sps.output_layer_set)
            .is_none_or(|set| set.necessary & (1u64 << nal.layer_id) == 0)
        {
            return Err(invalid(
                "HEVC slice layer is not necessary in active output set",
            ));
        }
        let reference_mask=e.graph.reference_list_layers(nal.layer_id)?;
        Ok(Some(Layout { e, index, nal,reference_mask }))
    }
}
impl Layout<'_> {
    pub(super) fn reference_count(&self)->usize {self.reference_mask.count_ones() as usize}
    pub(super) fn direct_count(&self) -> usize {
        self.e.graph.direct[self.index].count_ones() as usize
    }
    pub(super) fn poc_present(&self) -> bool {
        !self.nal.is_idr()
            || self.nal.layer_id > 0 && !self.e.representations.poc_lsb_not_present[self.index]
    }
    pub(super) fn extra(&self, b: &mut BitReader<'_>, bits: u8) -> Result<LayeredSliceSyntax> {
        let discardable = bits > 0 && b.bit()?;
        let cross_layer_bla = bits > 1 && b.bit()?;
        b.skip(usize::from(bits.saturating_sub(2)))?;
        if discardable && matches!(self.nal.unit_type, 1 | 3 | 5 | 7 | 9) {
            return Err(invalid("HEVC reference NAL cannot be discardable"));
        }
        if cross_layer_bla && (!self.nal.is_idr() || self.direct_count() != 0) {
            return Err(invalid(
                "HEVC cross-layer BLA flag requires an independent IDR",
            ));
        }
        Ok(LayeredSliceSyntax {
            discardable,
            cross_layer_bla,
            illumination_compensation:None,
            camera:None,
            inter_layer_prediction: None,
            in_component_prediction: None,
            coded_count_minus1: None,
            reference_layers: Vec::new(),
            poc_lsb_present: self.poc_present(),
            poc_reset: Default::default(),
        })
    }
    pub(super) fn reset(
        &self,
        context: &LayeredSliceContext<'_>,
        syntax: &LayeredSliceSyntax,
        extension: &[u8],
        slice_lsb: u32,
    ) -> Result<super::super::hevc_poc::ResetSyntax> {
        super::super::hevc_poc::ResetSyntax::parse_context(
            extension,
            super::super::hevc_poc::ResetContext {
                reset_info_present: context.pps.pps.poc_reset_info_present,
                poc_bits: context.sps.sps.poc_bits,
                nal: self.nal,
                lsb_aligned: self.e.representations.poc_lsb_aligned,
                direct_reference_layers: self.direct_count(),
                discardable: syntax.discardable,
                poc_lsb_absent: self.e.representations.poc_lsb_not_present[self.index],
                slice_lsb,
            },
        )
    }
    pub(super) fn references(
        &self,
        b: &mut BitReader<'_>,
        syntax: &mut LayeredSliceSyntax,
        budget: usize,
    ) -> Result<()> {
        let mut direct = [0u8; 63];
        let mut eligible = [0u8; 63];
        let mut count = 0;
        let mut eligible_count = 0;
        for i in 0..self.index {
            let id = self.e.graph.layer_ids[i];
            if self.reference_mask & (1u64 << id) == 0 {
                continue;
            }
            let limit = self.e.layer_sets.max_tid_ref_plus1[i][self.index];
            if limit > 7 {
                return Err(invalid("HEVC inter-layer temporal limit exceeds range"));
            }
            direct[count] = id;
            if self.e.layer_sets.sublayers_minus1[i] >= self.nal.temporal_id
                && (self.nal.temporal_id == 0 || limit > self.nal.temporal_id)
            {
                eligible[eligible_count] = count as u8;
                eligible_count += 1;
            }
            count += 1;
        }
        let default = self.e.layer_sets.default_ref_layers_active;
        let explicit = self.nal.layer_id > 0 && !default && count > 0;
        let enabled = if explicit {
            let flag = b.bit()?;
            syntax.inter_layer_prediction = Some(flag);
            flag
        } else {
            false
        };
        let width = (usize::BITS - count.saturating_sub(1).leading_zeros()) as u8;
        if enabled && count > 1 && !self.e.representations.max_one_active_reference {
            let value = b.read(width)? as usize;
            if value >= count {
                return Err(invalid(
                    "HEVC coded inter-layer reference count exceeds direct layers",
                ));
            }
            syntax.coded_count_minus1 = Some(value as u8);
        }
        let active = if self.nal.layer_id == 0 || eligible_count == 0 {
            0
        } else if default {
            eligible_count
        } else if !enabled {
            0
        } else if self.e.representations.max_one_active_reference || count == 1 {
            1
        } else {
            usize::from(syntax.coded_count_minus1.unwrap()) + 1
        };
        if self.e.representations.max_one_active_reference && active > 1 {
            return Err(invalid(
                "HEVC active inter-layer reference count violates VPS limit",
            ));
        }
        if active + std::mem::size_of::<Vec<u8>>() > budget {
            return Err(invalid(
                "HEVC inter-layer slice metadata exceeds memory budget",
            ));
        }
        syntax.reference_layers = Vec::with_capacity(active);
        let coded_indices = enabled && count > 1 && active != count;
        let mut previous = None;
        for i in 0..active {
            let selected = if coded_indices {
                b.read(width)? as usize
            } else if i < eligible_count {
                usize::from(eligible[i])
            } else {
                return Err(invalid("HEVC inferred inter-layer reference is absent"));
            };
            if selected >= count || previous.is_some_and(|p| selected <= p) {
                return Err(invalid("HEVC inter-layer reference index/order is invalid"));
            }
            previous = Some(selected);
            let id = direct[selected];
            let ref_index = self.e.graph.layer_ids.binary_search(&id).unwrap();
            let limit = self.e.layer_sets.max_tid_ref_plus1[ref_index][self.index];
            if self.e.layer_sets.sublayers_minus1[ref_index] < self.nal.temporal_id
                || !(limit > self.nal.temporal_id || limit == 0 && self.nal.temporal_id == 0)
            {
                return Err(invalid(
                    "HEVC slice selects a temporally unavailable reference layer",
                ));
            }
            // At limit zero/TemporalId zero, DPB binding must additionally check
            // that the actual source picture is IRAP (F.7.4.7.1).
            syntax.reference_layers.push(id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod camera_header_tests {
    use super::*;
    fn plan(data:&[u8])->crate::codec::hevc_decoder::LayeredAccessUnit {
        use crate::codec::{hevc_decoder::{LayeredParameters,LayeredAccessUnit},hevc_poc::LayeredPocState};
        let mut input=crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data),Default::default()).unwrap();
        let p=LayeredParameters::from_configuration(&input.tracks()[0].configuration,64<<20).unwrap();
        let mut packet=Vec::new();input.read_packet(0,0,&mut packet).unwrap();
        LayeredAccessUnit::parse(&p,&LayeredPocState::default(),&packet,4,1,64<<20).unwrap()
    }
    #[test]
    fn camera_header_budget_and_failure_keep_caller_cursor_unchanged() {
        let plan=plan(include_bytes!("../../tests/fixtures/playback-errors/hevc-wedge/slice-camera-deferred.mp4"));
        let ctx=plan.pictures()[1].binding.context();
        let data=include_bytes!("../../tests/fixtures/playback-errors/hevc-wedge/slice-camera-values-prefixed.bin");
        let mut bits=BitReader::new(data);bits.skip(3).unwrap();
        let c=ctx.camera_parameters(&mut bits,4096).unwrap().unwrap();
        assert_eq!(c.pairs[0].values,[1,0,0,0]);assert_eq!(bits.position(),9);
        let needed=std::mem::size_of::<SliceCamera>()+c.pairs.capacity()*std::mem::size_of::<SliceCameraPair>();
        for budget in [0,needed-1] {
            let mut bits=BitReader::new(data);bits.skip(3).unwrap();
            assert!(ctx.camera_parameters(&mut bits,budget).is_err());assert_eq!(bits.position(),3);
        }
        let mut bits=BitReader::new(data);bits.skip(3).unwrap();ctx.camera_parameters(&mut bits,needed).unwrap();
        let mut bits=BitReader::new(&[]);assert!(ctx.camera_parameters(&mut bits,needed).is_err());assert_eq!(bits.position(),0);
    }
    #[test]
    fn static_camera_values_are_inferred_without_consuming_slice_bits() {
        let plan=plan(include_bytes!("../../tests/fixtures/playback-errors/hevc-wedge/camera-parameters-static.mp4"));
        let ctx=plan.pictures()[1].binding.context();
        let mut bits=BitReader::new(&[]);
        let c=ctx.camera_parameters(&mut bits,4096).unwrap().unwrap();
        assert_eq!(c.pairs[0].values,[3,-4,5,-6]);assert_eq!(c.precision,5);assert_eq!(bits.position(),0);
    }
}
