//! Stateful decoding of length-prefixed AVC access units using FVid codecs.
//! Accepts supported progressive and MBAFF I/P/B access units.
//! `decode_order` leaves timestamp association and display reordering to callers.
use super::{
    avc::{Pps, Sps},
    avc_dpb::ReferenceBuffer,
    avc_inter_picture::decode_inter_optional_slices_with_motion,
    avc_picture::{IntraPicture, decode_intra_slices},
    avc_poc::{FieldOrder, PocDecoder},
    avc_reference_motion::ReferenceMotionField,
    avc_slice::{MemoryOperation, SliceHeader, SliceType},
    config::{AvcConfig, NalUnits},
};
use crate::{Result, invalid};
use std::sync::Arc;

/// One DPB entry owns image and motion metadata together. Intra-only pictures
/// have no motion field; co-located lookup treats them as intra at every cell.
pub struct DecodedReferencePicture {
    pub picture: Arc<IntraPicture>,
    pub motion: Option<Arc<ReferenceMotionField>>,
    /// Post-marking field POCs, including MMCO 5 adjustment. Keep both values:
    /// the frame minimum alone is insufficient for MBAFF temporal direct.
    pub field_order: FieldOrder,
}

/// A selected field owns pixels and co-located motion together.
struct DecodedReferenceField {
    field: Arc<super::avc_field_picture::PcmField>,
    motion: Option<Arc<ReferenceMotionField>>,
    motion_is_frame: bool,
}

/// Owns parameter sets, POC state and reference pictures. No external decoder.
/// The budget covers decoded reference storage and picture reconstruction;
/// input/configuration bytes and output Arcs retained by callers are excluded.
/// After an error, call `reset` before decoding another access unit.
#[derive(Clone)]
struct Parameters {
    sets: Vec<Sps>,
    pps_nals: Vec<(u32, Vec<u8>)>,
    pairs: Vec<(Sps, Pps)>,
}
pub struct AvcDecoder {
    length_size: u8,
    params: Parameters,
    initial_params: Parameters,
    decoded_sps: Option<Sps>,
    budget: usize,
    poc: PocDecoder,
    dpb: Option<ReferenceBuffer<DecodedReferencePicture>>,
    field_dpb: Option<super::avc_field_dpb::FieldBuffer<DecodedReferenceField>>,
    frame_field_dpb: Option<super::avc_field_dpb::FieldBuffer<DecodedReferenceField>>,
    pending_field: Option<(Arc<super::avc_field_picture::PcmField>, i32)>,
    field_pair_output: bool,
    active_sps: Option<u32>,
    previous_reference: Option<u32>,
    last_poc: Option<i32>,
    next_id: u64,
    failed: bool,
}
impl AvcDecoder {
    fn reference_pair_frame(
        top: &Arc<DecodedReferenceField>,
        bottom: &Arc<DecodedReferenceField>,
        field_order: FieldOrder,
        w: u32,
        h: u32,
        motion_bytes: usize,
    ) -> Result<Arc<DecodedReferencePicture>> {
        let full = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(3))
            .ok_or_else(|| invalid("AVC frame view geometry overflow"))?;
        let motion = if top.motion_is_frame && bottom.motion_is_frame {
            match (&top.motion, &bottom.motion) {
                (Some(a), Some(b)) if Arc::ptr_eq(a, b) => Some(Arc::clone(a)),
                (None, None) => None,
                _ => return Err(invalid("inconsistent migrated AVC frame motion")),
            }
        } else if !top.motion_is_frame && !bottom.motion_is_frame {
            if top.motion.is_none() && bottom.motion.is_none() {
                None
            } else {
                Some(Arc::new(ReferenceMotionField::weave_fields(
                    top.motion.as_deref(),
                    bottom.motion.as_deref(),
                    w as usize,
                    h as usize,
                    motion_bytes,
                )?))
            }
        } else {
            return Err(invalid("inconsistent complementary AVC motion origins"));
        };
        Ok(Arc::new(DecodedReferencePicture {
            picture: Arc::new(super::avc_field_picture::weave_pair(
                &top.field,
                &bottom.field,
                full,
            )?),
            motion,
            field_order,
        }))
    }
    pub fn new(configuration: &[u8], budget: usize) -> Result<Self> {
        let config = AvcConfig::parse(configuration)?;
        let sets: Vec<_> = config
            .sps
            .iter()
            .map(|nal| Sps::parse(nal))
            .collect::<Result<_>>()?;
        for (index, sps) in sets.iter().enumerate() {
            if sets[..index].iter().any(|previous| previous.id == sps.id) {
                return Err(invalid("duplicate AVC sequence parameter set ID"));
            }
        }
        let mut pairs = Vec::new();
        let mut pps_nals = Vec::new();
        for nal in config.pps {
            let pair = sets
                .iter()
                .find_map(|sps| Pps::parse(nal, sps).ok().map(|pps| (sps.clone(), pps)))
                .ok_or_else(|| invalid("invalid PPS or missing SPS"))?;
            if pairs.iter().any(|(_, p): &(Sps, Pps)| p.id == pair.1.id) {
                return Err(invalid("duplicate AVC parameter set ID"));
            }
            pps_nals.push((pair.1.id, nal.to_vec()));
            pairs.push(pair);
        }
        if pairs.is_empty() {
            return Err(invalid("AVC configuration has no parameter sets"));
        }
        let params = Parameters {
            sets,
            pps_nals,
            pairs,
        };
        Ok(Self {
            length_size: config.length_size,
            initial_params: params.clone(),
            params,
            decoded_sps: None,
            budget,
            poc: PocDecoder::new(),
            dpb: None,
            field_dpb: None,
            frame_field_dpb: None,
            pending_field: None,
            field_pair_output: false,
            active_sps: None,
            previous_reference: None,
            last_poc: None,
            next_id: 0,
            failed: false,
        })
    }
    pub fn active_vui(&self) -> Option<&super::avc::Vui> {
        self.params
            .pairs
            .iter()
            .find(|(s, _)| Some(s.id) == self.active_sps)
            .and_then(|(s, _)| s.vui.as_ref())
    }
    /// The VUI of the sequence parameter set the configuration record lists
    /// first. A coded picture names the set it wants, so [`Self::active_vui`]
    /// answers only once one has been decoded; what a stream says about its
    /// pictures before that is here, and a reader that grades the first picture
    /// asks at that moment.
    pub fn recorded_vui(&self) -> Option<&super::avc::Vui> {
        self.params.pairs.first().and_then(|(s, _)| s.vui.as_ref())
    }
    /// Drops reference pictures and requires the next coded picture to be IDR.
    pub fn reset(&mut self) {
        self.params.clone_from(&self.initial_params);
        self.decoded_sps = None;
        self.poc = PocDecoder::new();
        self.dpb = None;
        self.field_dpb = None;
        self.frame_field_dpb = None;
        self.pending_field = None;
        self.field_pair_output = false;
        self.active_sps = None;
        self.previous_reference = None;
        self.last_poc = None;
        self.next_id = 0;
        self.failed = false;
    }
    /// Decodes streams whose picture order counts strictly increase.
    /// For reordered B pictures, use [`Self::decode_order`] and reorder output
    /// using container timestamps; the MP4 playback reader does this automatically.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<Arc<IntraPicture>>> {
        if self.failed {
            return Err(invalid("AVC decoder requires reset after an error"));
        }
        self.field_pair_output = false;
        let result = self.decode_inner(packet, false);
        self.failed = result.is_err();
        result
    }
    /// Returns pictures in access-unit decode order, including B pictures.
    /// The caller must associate timestamps and perform display reordering.
    pub fn decode_order(&mut self, packet: &[u8]) -> Result<Option<Arc<IntraPicture>>> {
        if self.failed {
            return Err(invalid("AVC decoder requires reset after an error"));
        }
        self.field_pair_output = false;
        let result = self.decode_inner(packet, true);
        self.failed = result.is_err();
        result
    }
    /// A field pair emits one full picture when its second field arrives.
    pub fn output_is_field_pair(&self) -> bool {
        self.field_pair_output
    }
    pub fn has_pending_field(&self) -> bool {
        self.pending_field.is_some()
    }
    fn decode_fields(
        &mut self,
        headers: &[&SliceHeader],
        sps: &Sps,
        pps: &Pps,
        allow_reordering: bool,
    ) -> Result<Option<Arc<IntraPicture>>> {
        let header = headers[0];
        if headers
            .iter()
            .any(|h| !matches!(h.slice_type, SliceType::I | SliceType::P | SliceType::B))
        {
            return Err(crate::unsupported(
                "AVC inter field reconstruction is not connected",
            ));
        }
        if header.idr && self.pending_field.is_some() {
            return Err(crate::unsupported("unpaired AVC field before IDR"));
        }
        let (w, h) = sps.coded_dimensions();
        let full = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(3))
            .ok_or_else(|| invalid("AVC field storage overflow"))?;
        let motion_bytes = ReferenceMotionField::storage_bytes(w as usize, h as usize / 2)?;
        // Migrated full-frame maps retain MBAFF pair flags. A full map can
        // exceed two compact field maps by those flags; reserve the larger
        // representation without double-counting the Arc shared by parities.
        let full_motion_bytes = ReferenceMotionField::storage_bytes(w as usize, h as usize)?;
        let motion_per_reference = motion_bytes
            .checked_mul(2)
            .ok_or_else(|| invalid("AVC field motion storage overflow"))?
            .max(full_motion_bytes);
        let motion_reserve = motion_per_reference
            .checked_mul(sps.max_num_ref_frames.max(1) as usize)
            .and_then(|n| n.checked_add(motion_bytes))
            .ok_or_else(|| invalid("AVC field motion storage overflow"))?;
        let reserved = full
            .checked_mul(sps.max_num_ref_frames.max(1) as usize)
            .and_then(|n| n.checked_add(full))
            .and_then(|n| n.checked_add(full / 2))
            .and_then(|n| n.checked_add(motion_reserve))
            .ok_or_else(|| invalid("AVC field reference storage overflow"))?;
        let scratch = self
            .budget
            .checked_sub(reserved)
            .ok_or_else(|| invalid("AVC fields exceed decoder memory budget"))?;
        if !header.idr && self.field_dpb.is_none() {
            let frames = self
                .dpb
                .take()
                .ok_or_else(|| invalid("missing AVC frame references"))?;
            self.field_dpb = Some(frames.into_fields(|metadata, reference| {
                let split = |bottom| -> Result<Arc<DecodedReferenceField>> {
                    Ok(Arc::new(DecodedReferenceField {
                        field: Arc::new(super::avc_field_picture::split_frame(
                            &reference.picture,
                            bottom,
                            metadata.frame_num,
                            header.pps_id,
                            full / 2,
                        )?),
                        motion: reference.motion.clone(),
                        motion_is_frame: true,
                    }))
                };
                Ok(([split(false)?, split(true)?], reference.field_order))
            })?);
        }
        if !header.idr
            && let Some(previous) = self.previous_reference
        {
            let max = 1u32 << sps.frame_num_bits;
            if header.frame_num != previous && header.frame_num != (previous + 1) % max {
                if !sps.gaps_allowed {
                    return Err(invalid("AVC frame-number gap forbidden by SPS"));
                }
                let buffer = self
                    .field_dpb
                    .as_mut()
                    .ok_or_else(|| invalid("AVC gap requires initialized field DPB"))?;
                let mut missing = (previous + 1) % max;
                while missing != header.frame_num {
                    let order = self.poc.infer_nonexisting(sps, missing)?;
                    buffer.infer_nonexisting_fields(
                        missing,
                        order.map(|p| p.after_marking),
                        self.next_id,
                    )?;
                    self.next_id = self
                        .next_id
                        .checked_add(1)
                        .ok_or_else(|| invalid("AVC picture ID overflow"))?;
                    self.previous_reference = Some(missing);
                    missing = (missing + 1) % max;
                }
            }
        }
        let order = self.poc.decode(sps, header)?;
        let mut retained_motion = None;
        let mut field = if matches!(header.slice_type, SliceType::P | SliceType::B) {
            let dpb = self
                .field_dpb
                .as_ref()
                .ok_or_else(|| invalid("missing AVC field references"))?;
            let mut reference_ids = Vec::new();
            let mut direct_refs = Vec::new();
            let mut colocated = Vec::new();
            let mut references = Vec::new();
            references
                .try_reserve_exact(headers.len())
                .map_err(|_| invalid("cannot allocate AVC field reference contexts"))?;
            let implicit = header.slice_type == SliceType::B && pps.weighted_bipred == 2;
            let mut reference_orders = Vec::new();
            if implicit {
                reference_orders
                    .try_reserve_exact(headers.len())
                    .map_err(|_| invalid("cannot allocate AVC field orders"))?;
            }
            for h in headers {
                let lists = dpb.lists(h, order.before_marking.picture())?;
                if header.slice_type == SliceType::B {
                    let mut entries = [Vec::new(), Vec::new()];
                    for (target, list) in entries.iter_mut().zip([&lists.l0, &lists.l1]) {
                        for r in list {
                            let (poc, long_term) = dpb
                                .order(r.id, r.bottom)
                                .ok_or_else(|| invalid("missing direct field order"))?;
                            target.push(super::avc_direct::FieldDirectReference {
                                id: r.id,
                                bottom: r.bottom,
                                poc,
                                long_term,
                            });
                        }
                    }
                    let first = lists
                        .l1
                        .first()
                        .ok_or_else(|| invalid("empty direct field list1"))?;
                    let source = dpb
                        .get(first.id, first.bottom)
                        .ok_or_else(|| invalid("missing co-located field"))?;
                    colocated.push((source.motion.as_deref(), source.motion_is_frame));
                    direct_refs.push(entries);
                }
                reference_ids.push([
                    lists
                        .l0
                        .iter()
                        .map(|r| (r.id, r.bottom))
                        .collect::<Vec<_>>(),
                    lists
                        .l1
                        .iter()
                        .map(|r| (r.id, r.bottom))
                        .collect::<Vec<_>>(),
                ]);
                let mut resolved = [Vec::new(), Vec::new()];
                for (target, list) in resolved.iter_mut().zip([&lists.l0, &lists.l1]) {
                    target
                        .try_reserve_exact(list.len())
                        .map_err(|_| invalid("cannot allocate AVC active field references"))?;
                    for selected in list {
                        let reference = dpb
                            .get(selected.id, selected.bottom)
                            .ok_or_else(|| invalid("missing selected AVC field"))?;
                        let identity = selected
                            .id
                            .checked_mul(2)
                            .and_then(|v| v.checked_add(u64::from(selected.bottom)))
                            .ok_or_else(|| invalid("AVC field reference identity overflow"))?;
                        target.push((reference.field.as_ref(), identity));
                    }
                }
                if implicit {
                    let mut orders = [Vec::new(), Vec::new()];
                    for (target, list) in orders.iter_mut().zip([&lists.l0, &lists.l1]) {
                        target
                            .try_reserve_exact(list.len())
                            .map_err(|_| invalid("cannot allocate AVC reference field orders"))?;
                        for selected in list {
                            target.push(
                                dpb.order(selected.id, selected.bottom)
                                    .ok_or_else(|| invalid("missing selected AVC field order"))?,
                            );
                        }
                    }
                    reference_orders.push(orders);
                }
                references.push(resolved);
            }
            let contexts = references
                .iter()
                .map(|lists| [lists[0].as_slice(), lists[1].as_slice()])
                .collect::<Vec<_>>();
            let order_views = reference_orders
                .iter()
                .map(|lists| [lists[0].as_slice(), lists[1].as_slice()])
                .collect::<Vec<_>>();
            let weighting = super::avc_field_picture::ImplicitFieldWeights {
                poc: order.before_marking.picture(),
                references: &order_views,
            };
            let implicit_context = if implicit { Some(&weighting) } else { None };
            let direct_contexts = direct_refs
                .iter()
                .enumerate()
                .map(|(i, lists)| super::avc_direct::FieldDirectPrediction {
                    current_bottom: headers[i].bottom_field,
                    colocated_is_frame: colocated[i].1,
                    spatial: headers[i].direct_spatial_mv_pred,
                    inference8: sps.direct_8x8_inference,
                    current_poc: order.before_marking.picture(),
                    lists: [lists[0].as_slice(), lists[1].as_slice()],
                    colocated: colocated[i].0,
                })
                .collect::<Vec<_>>();
            let (picture, motion) = super::avc_field_picture::decode_inter_field_impl(
                headers,
                sps,
                pps,
                &contexts,
                implicit_context,
                if header.slice_type == SliceType::B {
                    Some(&direct_contexts)
                } else {
                    None
                },
                scratch,
                header.nal_ref_idc != 0,
            )?;
            if let Some(motion) = motion {
                // Reconstruction assigns slice IDs in macroblock address order,
                // independently of wire order (ASO).
                let mut sorted = (0..headers.len()).collect::<Vec<_>>();
                sorted.sort_by_key(|&i| headers[i].first_mb);
                let mappings = sorted
                    .iter()
                    .enumerate()
                    .map(|(slice, &i)| {
                        (
                            slice as u32,
                            [
                                reference_ids[i][0].as_slice(),
                                reference_ids[i][1].as_slice(),
                            ],
                        )
                    })
                    .collect::<Vec<_>>();
                retained_motion = Some(motion.snapshot_field_slices(&mappings, motion_bytes)?);
            }
            picture
        } else {
            super::avc_field_picture::decode_intra_slices(headers, sps, pps, scratch)?
        };
        if header.idr {
            self.field_dpb = Some(super::avc_field_dpb::FieldBuffer::new(
                sps.frame_num_bits,
                sps.max_num_ref_frames,
            )?);
            self.dpb = None;
            self.active_sps = Some(sps.id);
            self.decoded_sps = Some(sps.clone());
            self.previous_reference = None;
            self.last_poc = None;
        }
        if header
            .memory_operations
            .iter()
            .any(|op| matches!(op, MemoryOperation::Reset))
        {
            field.frame_num = 0;
        }
        let field = Arc::new(field);
        let output = if let Some((first, first_poc)) = &self.pending_field {
            let poc = (*first_poc).min(order.after_marking.picture());
            if !allow_reordering && self.last_poc.is_some_and(|last| poc <= last) {
                return Err(crate::unsupported(
                    "AVC field pair requires increasing picture order",
                ));
            }
            Some((
                Arc::new(super::avc_field_picture::weave_pair(first, &field, full)?),
                poc,
            ))
        } else {
            None
        };
        self.field_dpb
            .as_mut()
            .ok_or_else(|| invalid("AVC field DPB missing"))?
            .finish(
                header,
                order.after_marking.picture(),
                self.next_id,
                Arc::new(DecodedReferenceField {
                    field: Arc::clone(&field),
                    motion: retained_motion.map(Arc::new),
                    motion_is_frame: false,
                }),
            )?;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid("AVC field picture ID overflow"))?;
        if header.nal_ref_idc != 0 {
            self.previous_reference = Some(field.frame_num);
        }
        if let Some((picture, poc)) = output {
            self.pending_field = None;
            self.last_poc = Some(poc);
            self.field_pair_output = true;
            Ok(Some(picture))
        } else {
            self.pending_field = Some((field, order.after_marking.picture()));
            Ok(None)
        }
    }
    fn updated_parameters(&self, packet: &[u8]) -> Result<Option<Parameters>> {
        let mut updated: Option<Parameters> = None;
        let mut seen_slice = false;
        for nal in NalUnits::new(packet, self.length_size)? {
            let nal = nal?;
            let kind = nal[0] & 31;
            seen_slice |= matches!(kind, 1 | 5);
            let params = updated.as_ref().unwrap_or(&self.params);
            match kind {
                7 => {
                    let new = Sps::parse(nal)?;
                    if params.sets.iter().any(|s| *s == new) {
                        continue;
                    }
                    if seen_slice {
                        return Err(invalid("AVC changed SPS follows picture slices"));
                    }
                    let params = updated.get_or_insert_with(|| self.params.clone());
                    if let Some(old) = params.sets.iter_mut().find(|s| s.id == new.id) {
                        *old = new;
                    } else {
                        params.sets.push(new);
                    }
                }
                8 => {
                    let new = params
                        .sets
                        .iter()
                        .find_map(|s| Pps::parse(nal, s).ok())
                        .ok_or_else(|| invalid("AVC in-band PPS has no valid SPS"))?;
                    if params
                        .pps_nals
                        .iter()
                        .any(|(id, bytes)| *id == new.id && bytes == nal)
                    {
                        continue;
                    }
                    if seen_slice {
                        return Err(invalid("AVC changed PPS follows picture slices"));
                    }
                    let params = updated.get_or_insert_with(|| self.params.clone());
                    if let Some(old) = params.pps_nals.iter_mut().find(|(id, _)| *id == new.id) {
                        old.1 = nal.to_vec();
                    } else {
                        params.pps_nals.push((new.id, nal.to_vec()));
                    }
                }
                _ => {}
            }
        }
        if let Some(params) = &mut updated {
            params.pairs = params
                .pps_nals
                .iter()
                .map(|(_, nal)| {
                    params
                        .sets
                        .iter()
                        .find_map(|s| Pps::parse(nal, s).ok().map(|p| (s.clone(), p)))
                        .ok_or_else(|| invalid("AVC updated PPS has no valid SPS"))
                })
                .collect::<Result<_>>()?;
        }
        Ok(updated)
    }
    fn decode_inner(
        &mut self,
        packet: &[u8],
        allow_reordering: bool,
    ) -> Result<Option<Arc<IntraPicture>>> {
        if let Some(params) = self.updated_parameters(packet)? {
            self.params = params;
        }
        let mut coded = None;
        for nal in NalUnits::new(packet, self.length_size)? {
            let nal = nal?;
            match nal[0] & 31 {
                1 | 5 => {
                    coded.get_or_insert(nal);
                }
                6 | 7 | 8 | 9 | 10 | 11 | 12 => {}
                _ => return Err(invalid("unsupported in-band AVC NAL")),
            }
        }
        let Some(nal) = coded else {
            return Ok(None);
        };
        let id = SliceHeader::parameter_set_id(nal)?;
        let (sps, pps) = self
            .params
            .pairs
            .iter()
            .find(|(_, p)| p.id == id)
            .ok_or_else(|| invalid("unknown AVC PPS"))?;
        let slices =
            super::avc_access_unit::prepare(packet, self.length_size, sps, pps, self.budget)?;
        let header = &slices
            .first()
            .ok_or_else(|| invalid("missing AVC slice"))?
            .header;
        if !matches!(
            header.slice_type,
            SliceType::I | SliceType::P | SliceType::Sp | SliceType::Si | SliceType::B
        ) {
            return Err(crate::unsupported("AVC picture type is not implemented"));
        }
        if slices.iter().any(|slice| matches!(slice.header.slice_type, SliceType::Sp | SliceType::Si))
            && (!sps.frame_mbs_only || sps.chroma_format != 1 || sps.separate_colour_plane
                || sps.bit_depth_luma != 8 || sps.bit_depth_chroma != 8 || pps.cabac
                || sps.profile != 88 || pps.transform_8x8 || sps.transform_bypass)
        {
            return Err(crate::unsupported("AVC SP/SI requires progressive eight-bit 4:2:0 CAVLC"));
        }
        if !header.idr
            && (self.active_sps != Some(sps.id) || self.decoded_sps.as_ref() != Some(sps))
        {
            return Err(invalid("AVC stream/configuration change requires IDR"));
        }
        if header.field_pic {
            if let Some(canonical) = self.frame_field_dpb.take() {
                self.field_dpb = Some(canonical);
                self.dpb = None;
            }
            let sps = sps.clone();
            let pps = pps.clone();
            return self.decode_fields(
                &slices.iter().map(|s| &s.header).collect::<Vec<_>>(),
                &sps,
                &pps,
                allow_reordering,
            );
        }
        if self.pending_field.is_some() {
            return Err(crate::unsupported(
                "unpaired AVC field before frame picture",
            ));
        }
        if !header.idr
            && self
                .field_dpb
                .as_ref()
                .is_some_and(|b| b.has_frame_ineligible_stores())
        {
            self.frame_field_dpb = self.field_dpb.take();
        }
        if header.idr {
            self.frame_field_dpb = None;
        }
        if self.field_dpb.is_some() {
            if self.pending_field.is_some() {
                return Err(crate::unsupported(
                    "unpaired AVC field before frame picture",
                ));
            }
            if !header.idr {
                let (w, h) = sps.coded_dimensions();
                let full = (w as usize)
                    .checked_mul(h as usize)
                    .and_then(|n| n.checked_mul(3))
                    .ok_or_else(|| invalid("AVC migrated frame geometry overflow"))?;
                let motion_bytes = ReferenceMotionField::storage_bytes(w as usize, h as usize)?;
                let reserved = full
                    .checked_add(motion_bytes)
                    .and_then(|n| n.checked_mul(sps.max_num_ref_frames.max(1) as usize))
                    .and_then(|n| n.checked_add(full))
                    .and_then(|n| n.checked_add(motion_bytes))
                    .ok_or_else(|| invalid("AVC migrated frame memory overflow"))?;
                if reserved > self.budget {
                    return Err(invalid("AVC migrated frames exceed decoder memory budget"));
                }
                let fields = self.field_dpb.take().unwrap();
                self.dpb = Some(fields.into_frames(|top, bottom, field_order| {
                    Self::reference_pair_frame(top, bottom, field_order, w, h, motion_bytes)
                })?);
            } else {
                self.field_dpb = None;
            }
        }
        if !header.idr
            && let Some(previous) = self.previous_reference
        {
            let maximum = 1u32 << sps.frame_num_bits;
            if header.frame_num != previous && header.frame_num != (previous + 1) % maximum {
                if !sps.gaps_allowed {
                    return Err(invalid("AVC frame-number gap forbidden by SPS"));
                }
                let mut missing = (previous + 1) % maximum;
                while missing != header.frame_num {
                    let order = self.poc.infer_nonexisting(sps, missing)?;
                    if let Some(canonical) = self.frame_field_dpb.as_mut() {
                        canonical.infer_nonexisting_fields(
                            missing,
                            order.map(|p| p.after_marking),
                            self.next_id,
                        )?;
                    } else {
                        self.dpb
                            .as_mut()
                            .ok_or_else(|| invalid("AVC gap requires initialized DPB"))?
                            .infer_nonexisting_fields(
                                missing,
                                order.map(|p| p.after_marking),
                                self.next_id,
                            )?;
                    }
                    self.next_id = self
                        .next_id
                        .checked_add(1)
                        .ok_or_else(|| invalid("AVC picture ID overflow"))?;
                    self.previous_reference = Some(missing);
                    missing = (missing + 1) % maximum;
                }
            }
        }
        let (w, h) = sps.coded_dimensions();
        let motion_bytes = ReferenceMotionField::storage_bytes(w as usize, h as usize)?;
        let reference_bytes = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(3))
            .and_then(|n| n.checked_add(motion_bytes))
            .and_then(|n| n.checked_mul(sps.max_num_ref_frames.max(1) as usize))
            .ok_or_else(|| invalid("AVC reference memory overflow"))?;
        let reference_bytes = if self.frame_field_dpb.is_some() {
            reference_bytes
                .checked_mul(2)
                .ok_or_else(|| invalid("canonical AVC frame view memory overflow"))?
        } else {
            reference_bytes
        };
        let scratch_budget = self
            .budget
            .checked_sub(reference_bytes)
            .ok_or_else(|| invalid("AVC references exceed decoder memory budget"))?;
        if let Some(canonical) = self.frame_field_dpb.as_ref() {
            self.dpb =
                Some(canonical.frame_view(|a, b, o| {
                    Self::reference_pair_frame(a, b, o, w, h, motion_bytes)
                })?);
        }
        let order = self.poc.decode(sps, &header)?;
        if header.idr {
            self.dpb = Some(ReferenceBuffer::new(
                sps.frame_num_bits,
                sps.max_num_ref_frames,
            )?);
            self.active_sps = Some(sps.id);
            self.decoded_sps = Some(sps.clone());
            self.last_poc = None;
            self.previous_reference = None;
        }
        if !allow_reordering
            && self
                .last_poc
                .is_some_and(|p| order.before_marking.picture() <= p)
        {
            return Err(crate::unsupported(
                "AVC decode requires increasing picture order; use decode_order for reordered pictures",
            ));
        }
        let buffer = self
            .dpb
            .as_mut()
            .ok_or_else(|| invalid("AVC stream must begin with IDR"))?;
        let (picture, motion) = match header.slice_type {
            SliceType::I | SliceType::Si if slices.iter().all(|s| matches!(s.header.slice_type, SliceType::I | SliceType::Si)) => (
                decode_intra_slices(
                    &slices.iter().map(|slice| &slice.header).collect::<Vec<_>>(),
                    sps,
                    pps,
                    scratch_budget,
                )?,
                None,
            ),
            SliceType::I | SliceType::P | SliceType::Sp | SliceType::B => {
                let lists = slices
                    .iter()
                    .map(|slice| buffer.lists(&slice.header, order.before_marking.picture()))
                    .collect::<Result<Vec<_>>>()?;
                let entries = buffer.references();
                let mut refs_by_slice = Vec::with_capacity(slices.len());
                let mut metadata_by_slice = Vec::with_capacity(slices.len());
                for lists in &lists {
                    let mut refs = [Vec::new(), Vec::new()];
                    let mut metadata = [Vec::new(), Vec::new()];
                    for (list, ids) in [&lists.l0, &lists.l1].into_iter().enumerate() {
                        for id in ids {
                            refs[list].push(buffer.get(*id).map(|r| r.picture.as_ref()));
                            metadata[list].push(
                                *entries
                                    .iter()
                                    .find(|r| r.id == *id)
                                    .ok_or_else(|| invalid("missing AVC reference metadata"))?,
                            );
                        }
                    }
                    refs_by_slice.push(refs);
                    metadata_by_slice.push(metadata);
                }
                let direct_by_slice = slices
                    .iter()
                    .zip(&lists)
                    .zip(&metadata_by_slice)
                    .map(|((slice, lists), metadata)| {
                        if slice.header.slice_type != SliceType::B {
                            return Ok(None);
                        }
                        let first = lists
                            .l1
                            .first()
                            .ok_or_else(|| invalid("B slice has no L1 reference"))?;
                        Ok(Some(super::avc_direct::DirectPrediction {
                            spatial: slice.header.direct_spatial_mv_pred,
                            inference8: sps.direct_8x8_inference,
                            current_poc: order.before_marking.picture(),
                            list0: &metadata[0],
                            list1: &metadata[1],
                            colocated_field_pocs: {
                                let source = buffer
                                    .get(*first)
                                    .ok_or_else(|| invalid("missing AVC co-located order"))?;
                                [
                                    source
                                        .field_order
                                        .top
                                        .unwrap_or(source.field_order.picture()),
                                    source
                                        .field_order
                                        .bottom
                                        .unwrap_or(source.field_order.picture()),
                                ]
                            },
                            colocated: buffer
                                .get(*first)
                                .ok_or_else(|| invalid("AVC co-located picture is non-existing"))?
                                .motion
                                .as_deref(),
                        }))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let retain = header.nal_ref_idc != 0;
                let reconstruction_budget = scratch_budget
                    .checked_sub(if retain { motion_bytes } else { 0 })
                    .ok_or_else(|| invalid("AVC motion snapshot exceeds decoder budget"))?;
                let slice_headers = slices.iter().map(|s| &s.header).collect::<Vec<_>>();
                let reference_views = refs_by_slice
                    .iter()
                    .map(|refs| [refs[0].as_slice(), refs[1].as_slice()])
                    .collect::<Vec<_>>();
                let (picture, working) =
                    if sps.mb_adaptive_frame_field && !sps.frame_mbs_only && !header.field_pic {
                        let orders = lists
                            .iter()
                            .zip(&slices)
                            .map(|(lists, slice)| {
                                let mut result = [Vec::new(), Vec::new()];
                                if slice.header.slice_type != SliceType::B {
                                    return Ok(result);
                                }
                                for (list, ids) in [&lists.l0, &lists.l1].into_iter().enumerate() {
                                    for id in ids {
                                        result[list].push(
                                            buffer
                                                .get(*id)
                                                .map(|r| r.field_order)
                                                .or_else(|| buffer.inferred_field_order(*id))
                                                .ok_or_else(|| {
                                                    invalid("missing MBAFF field POC reference")
                                                })?,
                                        );
                                    }
                                }
                                Ok(result)
                            })
                            .collect::<Result<Vec<_>>>()?;
                        let contexts = slices
                            .iter()
                            .zip(&lists)
                            .zip(&metadata_by_slice)
                            .zip(&orders)
                            .map(|(((slice, lists), metadata), orders)| {
                                if slice.header.slice_type != SliceType::B {
                                    return Ok(None);
                                }
                                let id = lists
                                    .l1
                                    .first()
                                    .ok_or_else(|| invalid("MBAFF B slice has no L1 reference"))?;
                                Ok(Some(super::avc_direct::MbaffDirectPrediction {
                                    spatial: slice.header.direct_spatial_mv_pred,
                                    inference8: sps.direct_8x8_inference,
                                    current_order: order.before_marking,
                                    list0: &metadata[0],
                                    list1: &metadata[1],
                                    list0_orders: &orders[0],
                                    list1_orders: &orders[1],
                                    colocated: buffer
                                        .get(*id)
                                        .ok_or_else(|| {
                                            invalid("MBAFF co-located picture is non-existing")
                                        })?
                                        .motion
                                        .as_deref(),
                                }))
                            })
                            .collect::<Result<Vec<_>>>()?;
                        super::avc_mbaff_picture::decode_optional_inter_slices(
                            &slice_headers,
                            sps,
                            pps,
                            &reference_views,
                            &contexts.iter().map(Option::as_ref).collect::<Vec<_>>(),
                            reconstruction_budget,
                        )?
                    } else {
                        decode_inter_optional_slices_with_motion(
                            &slice_headers,
                            sps,
                            pps,
                            &reference_views,
                            &direct_by_slice
                                .iter()
                                .map(Option::as_ref)
                                .collect::<Vec<_>>(),
                            reconstruction_budget,
                        )?
                    };
                let motion = if retain {
                    let mappings = lists
                        .iter()
                        .enumerate()
                        .map(|(i, lists)| (i as u32, [lists.l0.as_slice(), lists.l1.as_slice()]))
                        .collect::<Vec<_>>();
                    Some(if sps.mb_adaptive_frame_field && !header.field_pic {
                        working.snapshot_mbaff_slices(&mappings, motion_bytes)?
                    } else {
                        working.snapshot_slices(&mappings, motion_bytes)?
                    })
                } else {
                    None
                };
                (picture, motion)
            }
            _ => unreachable!(),
        };
        let picture = Arc::new(picture);
        if let Some(canonical) = self.frame_field_dpb.as_mut() {
            self.dpb = None; // release temporary woven reference views first
            let motion = motion.map(Arc::new);
            let retained = if header.nal_ref_idc != 0 {
                let number = if header
                    .memory_operations
                    .iter()
                    .any(|o| matches!(o, MemoryOperation::Reset))
                {
                    0
                } else {
                    header.frame_num
                };
                let full = (w as usize)
                    .checked_mul(h as usize)
                    .and_then(|n| n.checked_mul(3))
                    .ok_or_else(|| invalid("canonical AVC frame geometry overflow"))?;
                Some(
                    [false, true]
                        .map(|bottom| {
                            super::avc_field_picture::split_frame(
                                &picture,
                                bottom,
                                number,
                                header.pps_id,
                                full / 2,
                            )
                            .map(|field| {
                                Arc::new(DecodedReferenceField {
                                    field: Arc::new(field),
                                    motion: motion.clone(),
                                    motion_is_frame: true,
                                })
                            })
                        })
                        .into_iter()
                        .collect::<Result<Vec<_>>>()?
                        .try_into()
                        .map_err(|_| invalid("canonical AVC field pair missing"))?,
                )
            } else {
                None
            };
            canonical.finish_frame(header, order.after_marking, self.next_id, retained)?;
        } else {
            buffer.finish(
                &header,
                order.after_marking.picture(),
                self.next_id,
                Arc::new(DecodedReferencePicture {
                    picture: picture.clone(),
                    motion: motion.map(Arc::new),
                    field_order: order.after_marking,
                }),
            )?;
        }
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid("AVC picture ID overflow"))?;
        self.last_poc = Some(order.after_marking.picture());
        if header.nal_ref_idc != 0 {
            self.previous_reference = Some(
                if header
                    .memory_operations
                    .iter()
                    .any(|op| matches!(op, MemoryOperation::Reset))
                {
                    0
                } else {
                    header.frame_num
                },
            );
        }
        Ok(Some(picture))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn configuration() -> Vec<u8> {
        let hex = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        let sps: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut data = vec![1, 66, 0xc0, 31, 0xff, 0xe1];
        data.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        data.extend_from_slice(&sps);
        data.extend_from_slice(&[1, 0, 4, 0x68, 0xce, 0x09, 0xc8]);
        data
    }
    fn packet(nals: &[&[u8]]) -> Vec<u8> {
        let mut data = Vec::new();
        for nal in nals {
            data.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            data.extend_from_slice(nal);
        }
        data
    }
    #[test]
    fn canonical_fields_keep_identity_through_frame_views_and_real_later_prediction() {
        use crate::container::mp4::Mp4Reader;
        use std::io::Cursor;
        for (video, number, bottom, long) in [
            (
                &include_bytes!(
                    "../../tests/fixtures/playback-errors/avc-unified-field-partial-8bit-top-motion-coded.mp4"
                )[..],
                0,
                true,
                false,
            ),
            (
                &include_bytes!(
                    "../../tests/fixtures/playback-errors/avc-unified-field-partial-10bit-bottom-motion-skip-aso.mp4"
                )[..],
                0,
                false,
                false,
            ),
            (
                &include_bytes!(
                    "../../tests/fixtures/playback-errors/avc-unified-field-mixed-8bit-top-motion-skip.mp4"
                )[..],
                1,
                false,
                true,
            ),
            (
                &include_bytes!(
                    "../../tests/fixtures/playback-errors/avc-unified-field-mixed-10bit-bottom-motion-coded-aso.mp4"
                )[..],
                1,
                true,
                true,
            ),
        ] {
            let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
            let config = input.tracks()[0].configuration.clone();
            let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
            for sample in 0..4 {
                let mut packet = Vec::new();
                input.read_packet(0, sample, &mut packet).unwrap();
                decoder.decode_order(&packet).unwrap();
            }
            let owner = decoder.field_dpb.as_ref().unwrap();
            let target = owner
                .references()
                .into_iter()
                .find(|r| r.frame_num == number)
                .unwrap();
            let weak = Arc::downgrade(owner.get(target.id, bottom).unwrap());
            let order = owner.order(target.id, bottom).unwrap();
            assert_eq!(order.1, long);
            let mut packet = Vec::new();
            input.read_packet(0, 4, &mut packet).unwrap();
            assert!(decoder.decode_order(&packet).unwrap().is_some());
            assert!(decoder.dpb.is_none());
            assert!(decoder.field_dpb.is_none());
            let owner = decoder.frame_field_dpb.as_ref().unwrap();
            assert_eq!(owner.order(target.id, bottom), Some(order));
            assert!(weak.ptr_eq(&Arc::downgrade(owner.get(target.id, bottom).unwrap())));
            for sample in 5..7 {
                input.read_packet(0, sample, &mut packet).unwrap();
                decoder.decode_order(&packet).unwrap();
            }
            let owner = decoder.field_dpb.as_ref().unwrap();
            assert_eq!(owner.order(target.id, bottom), Some(order));
            let current = owner
                .references()
                .into_iter()
                .find(|r| r.frame_num == 3)
                .unwrap();
            for parity in [false, true] {
                let cell = owner
                    .get(current.id, parity)
                    .unwrap()
                    .motion
                    .as_ref()
                    .unwrap()
                    .colocated([0, 0])
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    (cell.picture_id, cell.reference_bottom_field, cell.vector),
                    (target.id, Some(bottom), [0, 0])
                );
            }
            input.read_packet(0, 7, &mut packet).unwrap();
            assert!(decoder.decode_order(&packet).unwrap().is_some());
            let owner = decoder.frame_field_dpb.as_ref().unwrap();
            assert_eq!(owner.order(target.id, bottom), Some(order));
            assert!(weak.upgrade().is_some());
            decoder.reset();
            assert!(weak.upgrade().is_none());
            assert!(decoder.frame_field_dpb.is_none());
        }
    }
    #[test]
    fn native_field_migration_keeps_marking_motion_and_releases_old_stores() {
        use crate::container::mp4::Mp4Reader;
        use std::io::Cursor;
        for video in [
            &include_bytes!(
                "../../tests/fixtures/playback-errors/avc-field-frame-paff-8bit-top-long-p-coded-motion.mp4"
            )[..],
            &include_bytes!(
                "../../tests/fixtures/playback-errors/avc-field-frame-mbaff-8bit-top-long-temporal-coded-motion.mp4"
            )[..],
            &include_bytes!(
                "../../tests/fixtures/playback-errors/avc-field-frame-paff-10bit-bottom-short-temporal-skip-motion.mp4"
            )[..],
            &include_bytes!(
                "../../tests/fixtures/playback-errors/avc-field-frame-mbaff-10bit-bottom-short-spatial-coded-motion.mp4"
            )[..],
        ] {
            let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
            let count = input.tracks()[0].samples.len();
            let config = input.tracks()[0].configuration.clone();
            let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
            for sample in 0..count - 1 {
                let mut packet = Vec::new();
                input.read_packet(0, sample, &mut packet).unwrap();
                decoder.decode_order(&packet).unwrap();
            }
            let fields = decoder.field_dpb.as_ref().unwrap();
            let saved = fields
                .references()
                .into_iter()
                .map(|r| {
                    let entries = [
                        fields.get(r.id, false).unwrap(),
                        fields.get(r.id, true).unwrap(),
                    ];
                    let weak = entries.map(Arc::downgrade);
                    let motion = entries.map(|e| {
                        [0, 16]
                            .map(|x| e.motion.as_ref().and_then(|m| m.colocated([x, 0]).unwrap()))
                    });
                    (r, weak, motion)
                })
                .collect::<Vec<_>>();
            let mut packet = Vec::new();
            input.read_packet(0, count - 1, &mut packet).unwrap();
            assert!(decoder.decode_order(&packet).unwrap().is_some());
            assert!(decoder.field_dpb.is_none());
            let frames = decoder.dpb.as_ref().unwrap();
            for (old, weak, motion) in saved {
                let metadata = frames
                    .references()
                    .into_iter()
                    .find(|r| r.id == old.id)
                    .unwrap();
                assert_eq!(metadata.frame_num, old.frame_num);
                assert_eq!(
                    metadata.poc,
                    old.top_poc.unwrap().min(old.bottom_poc.unwrap())
                );
                assert_eq!(metadata.long_term_index, old.long_term_indices[0]);
                let entry = frames.get(old.id).unwrap();
                assert_eq!(
                    entry.field_order,
                    FieldOrder {
                        top: old.top_poc,
                        bottom: old.bottom_poc
                    }
                );
                for bottom in [false, true] {
                    for (i, x) in [0, 16].into_iter().enumerate() {
                        assert_eq!(
                            entry.motion.as_ref().and_then(|m| m
                                .colocated_for_field([x, 0], bottom)
                                .unwrap()
                                .motion),
                            motion[usize::from(bottom)][i]
                        );
                    }
                }
                assert!(weak.iter().all(|w| w.upgrade().is_none()));
            }
            decoder.reset();
            assert!(decoder.dpb.is_none());
            assert!(decoder.field_dpb.is_none());
        }
    }
    #[test]
    fn frame_migration_shares_motion_between_parities_and_reset_releases_it() {
        use crate::container::mp4::Mp4Reader;
        use std::io::Cursor;
        for (mode,video) in [
            &include_bytes!(
                "../../tests/fixtures/playback-errors/avc-frame-field-direct-paff-8bit-top-temporal-coded-motion.mp4"
            )[..],
            &include_bytes!(
                "../../tests/fixtures/playback-errors/avc-frame-field-direct-mbaff-8bit-bottom-temporal-coded-motion.mp4"
            )[..],
            &include_bytes!(
                "../../tests/fixtures/playback-errors/avc-frame-field-direct-field-10bit-top-temporal-coded-motion.mp4"
            )[..],
            &include_bytes!(
                "../../tests/fixtures/playback-errors/avc-frame-field-direct-mixed-10bit-bottom-temporal-coded-motion.mp4"
            )[..],
        ].into_iter().enumerate() {
            let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
            let config = input.tracks()[0].configuration.clone();
            let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
            for sample in 0..2 {
                let mut packet = Vec::new();
                input.read_packet(0, sample, &mut packet).unwrap();
                assert!(decoder.decode_order(&packet).unwrap().is_some());
            }
            let dpb = decoder.dpb.as_ref().unwrap();
            let reference = dpb
                .references()
                .into_iter()
                .find(|r| r.frame_num == 1)
                .unwrap();
            let motion = Arc::downgrade(dpb.get(reference.id).unwrap().motion.as_ref().unwrap());
            {
                let saved=motion.upgrade().unwrap();
                for x in [0,16] {
                    let field=mode==2 || (mode==3 && x==16);
                    let selected=saved.colocated_for_field([x,0],false).unwrap();
                    assert_eq!(selected.scale, if field {
                        super::super::avc_reference_motion::ColocatedScale::Same
                    } else {super::super::avc_reference_motion::ColocatedScale::FrameToField});
                    let cell=selected.motion.unwrap();assert_ne!(cell.vector,[0,0]);
                    assert_eq!(cell.reference_bottom_field,field.then_some(false));
                }
            }
            let mut packet = Vec::new();
            input.read_packet(0, 2, &mut packet).unwrap();
            assert!(decoder.decode_order(&packet).unwrap().is_none());
            assert!(decoder.dpb.is_none());
            let fields = decoder.field_dpb.as_ref().unwrap();
            for bottom in [false, true] {
                let entry = fields.get(reference.id, bottom).unwrap();
                assert!(entry.motion_is_frame);
                assert!(motion.ptr_eq(&Arc::downgrade(entry.motion.as_ref().unwrap())));
            }
            input.read_packet(0, 3, &mut packet).unwrap();
            assert!(decoder.decode_order(&packet).unwrap().is_some());
            decoder.reset();
            assert!(motion.upgrade().is_none());
        }
    }
    #[test]
    fn b_field_reproducer_requires_motion_context_and_native_decode_accepts_it() {
        use crate::container::mp4::Mp4Reader;
        use std::io::Cursor;
        for video in [
            &include_bytes!(
                "../../tests/fixtures/playback-errors/avc-field-b-direct-8bit-top-first-temporal-coded-cavlc-filter0-infer0.mp4"
            )[..],
            &include_bytes!(
                "../../tests/fixtures/playback-errors/avc-field-b-direct-10bit-bottom-first-spatial-skip-cabac-init2-filter2-infer1-aso.mp4"
            )[..],
        ] {
            let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
            let config = input.tracks()[0].configuration.clone();
            let avc = AvcConfig::parse(&config).unwrap();
            let sps = Sps::parse(avc.sps[0]).unwrap();
            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
            let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
            for sample in 0..8 {
                let mut packet = Vec::new();
                input.read_packet(0, sample, &mut packet).unwrap();
                if sample >= 6 {
                    let headers = NalUnits::new(&packet, avc.length_size)
                        .unwrap()
                        .map(|n| SliceHeader::parse(n.unwrap(), &sps, &pps).unwrap())
                        .collect::<Vec<_>>();
                    let dpb = decoder.field_dpb.as_ref().unwrap();
                    let selected = headers
                        .iter()
                        .map(|h| dpb.lists(h, h.poc_lsb.unwrap() as i32).unwrap())
                        .collect::<Vec<_>>();
                    for lists in &selected {
                        let first = lists.l1[0];
                        assert!(dpb.get(first.id, first.bottom).unwrap().motion.is_some());
                    }
                    let references = selected
                        .iter()
                        .map(|lists| {
                            [&lists.l0, &lists.l1].map(|list| {
                                list.iter()
                                    .map(|r| {
                                        (
                                            dpb.get(r.id, r.bottom).unwrap().field.as_ref(),
                                            r.id * 2 + u64::from(r.bottom),
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            })
                        })
                        .collect::<Vec<_>>();
                    let contexts = references
                        .iter()
                        .map(|l| [l[0].as_slice(), l[1].as_slice()])
                        .collect::<Vec<_>>();
                    let views = headers.iter().collect::<Vec<_>>();
                    let error =
                        super::super::avc_field_picture::decode_inter_field_lists_with_order(
                            &views,
                            &sps,
                            &pps,
                            &contexts,
                            None,
                            1 << 20,
                        )
                        .err()
                        .expect("legacy entrypoint lacks direct context");
                    assert!(
                        error.to_string().contains(
                            "B field direct prediction requires reference motion context"
                        ),
                        "{error}"
                    );
                }
                let picture = decoder.decode_order(&packet).unwrap();
                assert_eq!(picture.is_some(), sample % 2 == 1);
            }
        }
    }
    #[test]
    fn field_dpb_retains_slice_motion_and_reset_releases_it() {
        use crate::container::mp4::Mp4Reader;
        use std::io::Cursor;
        for (video, oracle) in [
            (
                &include_bytes!(
                    "../../tests/fixtures/playback-errors/avc-field-cabac-p-multiref-8bit-top-first-4x4-r1-list1-init0-filter0-aso.mp4"
                )[..],
                &include_bytes!(
                    "../../tests/fixtures/playback-errors/avc-field-cabac-p-multiref-8bit-top-first-4x4-r1-list1-init0-filter0-aso.yuv"
                )[..],
            ),
            (
                &include_bytes!(
                    "../../tests/fixtures/playback-errors/avc-field-cabac-p-multiref-10bit-bottom-first-4x4-r1-list1-init2-filter2-aso.mp4"
                )[..],
                &include_bytes!(
                    "../../tests/fixtures/playback-errors/avc-field-cabac-p-multiref-10bit-bottom-first-4x4-r1-list1-init2-filter2-aso.yuv"
                )[..],
            ),
            (
                &include_bytes!(
                    "../../tests/fixtures/playback-errors/avc-field-skip-8bit-top-first.mp4"
                )[..],
                &include_bytes!(
                    "../../tests/fixtures/playback-errors/avc-field-skip-8bit-top-first.yuv"
                )[..],
            ),
        ] {
            let mut reader = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
            let config = reader.tracks()[0].configuration.clone();
            let avc = AvcConfig::parse(&config).unwrap();
            let sps = Sps::parse(avc.sps[0]).unwrap();
            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
            let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
            for _ in 0..2 {
                let mut pixels = Vec::new();
                let mut retained = Vec::new();
                for sample in 0..reader.tracks()[0].samples.len() {
                    let mut packet = Vec::new();
                    reader.read_packet(0, sample, &mut packet).unwrap();
                    let mut headers = NalUnits::new(&packet, avc.length_size)
                        .unwrap()
                        .map(|n| SliceHeader::parse(n.unwrap(), &sps, &pps).unwrap())
                        .collect::<Vec<_>>();
                    headers.sort_by_key(|h| h.first_mb);
                    let selected = if headers[0].slice_type == SliceType::P {
                        headers
                            .iter()
                            .map(|h| {
                                decoder
                                    .field_dpb
                                    .as_ref()
                                    .unwrap()
                                    .lists(h, h.poc_lsb.unwrap() as i32)
                                    .unwrap()
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    if let Some(picture) = decoder.decode_order(&packet).unwrap() {
                        picture.write_planar(&mut pixels).unwrap();
                    }
                    let dpb = decoder.field_dpb.as_ref().unwrap();
                    let store = dpb
                        .references()
                        .into_iter()
                        .find(|r| r.frame_num == headers[0].frame_num)
                        .unwrap();
                    let decoded = dpb.get(store.id, headers[0].bottom_field).unwrap();
                    if headers[0].slice_type == SliceType::I {
                        assert!(decoded.motion.is_none());
                    } else {
                        let motion = decoded
                            .motion
                            .as_ref()
                            .expect("reference P field retains motion");
                        for y in (0..16).step_by(4) {
                            for x in (0..32).step_by(4) {
                                let address = x / 16;
                                let slice = headers
                                    .iter()
                                    .rposition(|h| h.first_mb as usize <= address)
                                    .unwrap();
                                let cell = motion.at([x, y]).unwrap();
                                assert!(cell[1].is_none());
                                let value = cell[0].unwrap();
                                let reference = selected[slice].l0[value.reference_index as usize];
                                assert_eq!(value.picture_id, reference.id);
                                assert_eq!(value.reference_bottom_field, Some(reference.bottom));
                            }
                        }
                        retained.push(Arc::downgrade(decoded));
                    }
                }
                assert_eq!(pixels, oracle);
                assert!(!retained.is_empty());
                decoder.reset();
                assert!(decoder.field_dpb.is_none());
                assert!(retained.iter().all(|weak| weak.upgrade().is_none()));
            }
        }
    }
    #[test]
    fn dpb_retains_motion_identity_and_releases_evicted_metadata() {
        fn hex(text: &str) -> Vec<u8> {
            (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
                .collect()
        }
        let sps = hex("674d400ada7a1000000300100000030320f1226a");
        let pps = hex("68ee06cb20");
        let mut config = vec![1, 77, 64, 10, 255, 225];
        config.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        config.extend_from_slice(&sps);
        config.push(1);
        config.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        config.extend_from_slice(&pps);
        let idr = packet(&[&hex("658884fffeedcfe052e36fc1")]);
        let p1 = packet(&[&hex("419a23ff5d2e09a431c3d011f0")]);
        let p2 = packet(&[&hex("419a43ff5d2e09a431c3d011f1")]);
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        assert_eq!(decoder.decode(&idr).unwrap().unwrap().y, vec![64; 256]);
        assert!(
            decoder
                .dpb
                .as_ref()
                .unwrap()
                .get(0)
                .unwrap()
                .motion
                .is_none()
        );
        assert_eq!(decoder.decode(&p1).unwrap().unwrap().y, vec![66; 256]);
        let old = decoder.dpb.as_ref().unwrap().get(1).unwrap();
        // Retained timing belongs to the same decoded picture as its motion;
        // reference-list reordering/eviction must not replace it with output POC.
        let retained_order = old.field_order;
        assert!(retained_order.top.is_some() && retained_order.bottom.is_some());
        assert_eq!(
            retained_order.picture(),
            decoder.dpb.as_ref().unwrap().references()[0].poc
        );
        let motion = old
            .motion
            .as_ref()
            .unwrap()
            .colocated([0, 0])
            .unwrap()
            .unwrap();
        assert_eq!(
            (motion.picture_id, motion.reference_index, motion.vector),
            (0, 0, [0, 0])
        );
        let weak = Arc::downgrade(old);
        assert_eq!(decoder.decode(&p2).unwrap().unwrap().y, vec![68; 256]);
        assert!(weak.upgrade().is_none());
        let dpb = decoder.dpb.as_ref().unwrap();
        assert_eq!(dpb.references().len(), 1);
        assert_eq!(
            dpb.get(2)
                .unwrap()
                .motion
                .as_ref()
                .unwrap()
                .colocated([0, 0])
                .unwrap()
                .unwrap()
                .picture_id,
            1
        );
        let weak = Arc::downgrade(dpb.get(2).unwrap());
        decoder.decode(&idr).unwrap();
        assert!(weak.upgrade().is_none());
        assert!(
            decoder
                .dpb
                .as_ref()
                .unwrap()
                .get(3)
                .unwrap()
                .motion
                .is_none()
        );
        decoder.reset();
        assert!(decoder.dpb.is_none());
    }
    #[test]
    fn errors_poison_until_reset_and_metadata_does_not_create_pictures() {
        let mut decoder = AvcDecoder::new(&configuration(), 1 << 20).unwrap();
        let aud = packet(&[&[9, 0xf0]]);
        assert!(decoder.decode(&aud).unwrap().is_none());
        assert!(decoder.decode(&[0, 0, 0, 10, 9]).is_err());
        assert!(decoder.decode(&aud).is_err());
        decoder.reset();
        assert!(decoder.decode(&aud).unwrap().is_none());
    }
    #[test]
    fn rejects_multiple_slices_before_changing_picture_state_and_bounds_references() {
        let idr = &[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80];
        let mut decoder = AvcDecoder::new(&configuration(), packet(&[idr, idr]).len()).unwrap();
        assert!(decoder.decode(&packet(&[idr, idr])).is_err());
        assert!(decoder.active_sps.is_none());
        decoder.reset();
        let error = decoder.decode(&packet(&[idr])).err().unwrap();
        assert!(error.to_string().contains("references exceed"));
        assert!(decoder.active_sps.is_none());
    }
}
