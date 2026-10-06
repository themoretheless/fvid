//! Progressive AVC reference-frame storage and marking (H.264 8.2.5).
//! Output/display reordering is separate; retain an `Arc` for pictures awaiting display.
use super::{
    avc_poc::FieldOrder,
    avc_references::{FrameReference, ReferenceLists, frame_lists},
    avc_slice::{MemoryOperation, RefModification, SliceHeader, SliceType},
};
use crate::{Result, invalid};
use std::sync::Arc;

#[derive(Clone)]
pub struct ReferenceBuffer<T> {
    frame_num_bits: u8,
    capacity: usize,
    max_long_term_index: Option<u32>,
    initialized: bool,
    // Inferred gap entries have no sample storage. Their POC is absent for
    // POC type 0, so B-list initialization must exclude those entries.
    frames: Vec<(FrameReference, Option<Arc<T>>, bool, Option<FieldOrder>)>,
}
impl<T> ReferenceBuffer<T> {
    pub fn new(frame_num_bits: u8, max_num_ref_frames: u32) -> Result<Self> {
        if !(4..=16).contains(&frame_num_bits) || max_num_ref_frames > 16 {
            return Err(invalid("invalid AVC reference buffer configuration"));
        }
        Ok(Self {
            frame_num_bits,
            capacity: max_num_ref_frames.max(1) as usize,
            max_long_term_index: None,
            initialized: false,
            frames: Vec::new(),
        })
    }
    /// Move frame marking state into parity-aware storage without changing IDs.
    pub(super) fn into_fields<U>(
        self,
        convert: impl Fn(
            super::avc_references::FrameReference,
            &Arc<T>,
        ) -> Result<([Arc<U>; 2], FieldOrder)>,
    ) -> Result<super::avc_field_dpb::FieldBuffer<U>> {
        super::avc_field_dpb::FieldBuffer::from_frame_storage(
            self.frame_num_bits,
            self.capacity,
            self.max_long_term_index,
            self.initialized,
            self.frames,
            convert,
        )
    }
    pub(super) fn from_field_storage(
        bits: u8,
        capacity: usize,
        limit: Option<u32>,
        initialized: bool,
        frames: Vec<(FrameReference, Option<Arc<T>>, bool, Option<FieldOrder>)>,
    ) -> Result<Self> {
        let mut result = Self::new(bits, capacity as u32)?;
        result.max_long_term_index = limit;
        result.initialized = initialized;
        result.frames = frames;
        Ok(result)
    }
    pub fn get(&self, id: u64) -> Option<&Arc<T>> {
        self.frames
            .iter()
            .find(|(r, _, _, _)| r.id == id)
            .and_then(|(_, p, _, _)| p.as_ref())
    }
    pub fn inferred_field_order(&self, id: u64) -> Option<FieldOrder> {
        self.frames
            .iter()
            .find(|(r, _, _, _)| r.id == id)
            .and_then(|(_, _, _, order)| *order)
    }
    /// Retain exact inferred field POCs alongside the pixel-free gap slot.
    pub fn infer_nonexisting_fields(
        &mut self,
        frame_num: u32,
        order: Option<FieldOrder>,
        id: u64,
    ) -> Result<()> {
        if order.is_some_and(|o| o.top.is_none() || o.bottom.is_none()) {
            return Err(invalid("AVC inferred frame requires both field POCs"));
        }
        self.infer_nonexisting(frame_num, order.map(FieldOrder::picture), id)?;
        self.frames
            .iter_mut()
            .find(|(r, _, _, _)| r.id == id)
            .expect("inserted gap slot")
            .3 = order;
        Ok(())
    }
    pub fn references(&self) -> Vec<FrameReference> {
        self.frames.iter().map(|(r, _, _, _)| *r).collect()
    }
    pub fn lists(&self, header: &SliceHeader, poc: i32) -> Result<ReferenceLists> {
        if header.field_pic {
            return Err(crate::unsupported(
                "AVC field reference lists are not implemented",
            ));
        }
        let max = 1u32 << self.frame_num_bits;
        if header.frame_num >= max {
            return Err(invalid("AVC reference list frame number out of range"));
        }
        if matches!(
            header.slice_type,
            SliceType::P | SliceType::Sp | SliceType::B
        ) {
            if !self
                .frames
                .iter()
                .any(|(_, picture, _, _)| picture.is_some())
            {
                return Err(invalid("AVC reference list has no existing picture"));
            }
            for commands in [&header.modifications_l0, &header.modifications_l1]
                .into_iter()
                .take(if header.slice_type == SliceType::B {
                    2
                } else {
                    1
                })
            {
                let mut predicted = i64::from(header.frame_num);
                for command in commands {
                    if let RefModification::Subtract(n) | RefModification::Add(n) = *command {
                        if n >= max {
                            return Err(invalid(
                                "AVC reference modification difference out of range",
                            ));
                        }
                        predicted = (predicted
                            + if matches!(command, RefModification::Subtract(_)) {
                                -(i64::from(n) + 1)
                            } else {
                                i64::from(n) + 1
                            })
                        .rem_euclid(i64::from(max));
                        if self.frames.iter().any(|(reference, picture, _, _)| {
                            picture.is_none()
                                && reference.long_term_index.is_none()
                                && i64::from(reference.frame_num) == predicted
                        }) {
                            return Err(invalid(
                                "AVC reference modification selects non-existing picture",
                            ));
                        }
                    }
                }
            }
        }
        let references: Vec<_> = self
            .frames
            .iter()
            .filter(|(_, _, has_poc, _)| header.slice_type != SliceType::B || *has_poc)
            .map(|(reference, _, _, _)| *reference)
            .collect();
        frame_lists(
            &references,
            self.frame_num_bits,
            header.frame_num,
            poc,
            header.slice_type,
            [header.refs_l0 as usize, header.refs_l1 as usize],
            [&header.modifications_l0, &header.modifications_l1],
        )
    }
    /// Mark one gap-inferred frame as non-existing short-term reference (8.2.5.2).
    /// It consumes a sliding-window slot but owns no pixels and cannot be used
    /// for prediction. `poc` is None only for pic_order_cnt_type 0. Validation
    /// or eviction failure leaves the buffer unchanged.
    pub fn infer_nonexisting(&mut self, frame_num: u32, poc: Option<i32>, id: u64) -> Result<()> {
        let max = 1u32 << self.frame_num_bits;
        if !self.initialized || frame_num >= max {
            return Err(invalid(
                "AVC inferred reference needs initialized DPB and valid frame number",
            ));
        }
        let mut frames = self.frames.clone();
        if frames.len() == self.capacity {
            let index = frames
                .iter()
                .enumerate()
                .filter(|(_, (r, _, _, _))| r.long_term_index.is_none())
                .min_by_key(|(_, (r, _, _, _))| wrapped(r.frame_num, frame_num, max))
                .map(|(i, _)| i)
                .ok_or_else(|| invalid("AVC sliding window has no short-term reference"))?;
            frames.remove(index);
        }
        if frames.len() >= self.capacity
            || frames.iter().any(|(r, _, _, _)| {
                r.id == id || (r.long_term_index.is_none() && r.frame_num == frame_num)
            })
        {
            return Err(invalid(
                "AVC inferred reference capacity or identity conflict",
            ));
        }
        frames.push((
            FrameReference {
                id,
                frame_num,
                poc: poc.unwrap_or(0),
                long_term_index: None,
            },
            None,
            poc.is_some(),
            None,
        ));
        self.frames = frames;
        Ok(())
    }
    /// Call after reconstructing every slice of a frame. `poc` is the POC after
    /// MMCO 5 adjustment from `PocDecoder`. Non-reference frames are not retained.
    /// The caller must handle frame-number gaps before decoding the current frame.
    /// Invalid marking leaves the buffer and its long-term limit unchanged.
    pub fn finish(
        &mut self,
        header: &SliceHeader,
        poc: i32,
        id: u64,
        picture: Arc<T>,
    ) -> Result<()> {
        if header.field_pic {
            return Err(crate::unsupported(
                "AVC field reference marking is not implemented",
            ));
        }
        let max = 1u32 << self.frame_num_bits;
        if header.frame_num >= max
            || header.nal_ref_idc > 3
            || (header.idr && (header.frame_num != 0 || header.nal_ref_idc == 0))
        {
            return Err(invalid("invalid AVC reference marking header"));
        }
        if !header.idr && !self.initialized {
            return Err(invalid("AVC reference buffer needs an IDR frame"));
        }
        if header.nal_ref_idc == 0 {
            return Ok(());
        }
        let mut frames = self.frames.clone();
        let mut limit = self.max_long_term_index;
        let mut current = FrameReference {
            id,
            frame_num: header.frame_num,
            poc,
            long_term_index: None,
        };
        if header.idr {
            frames.clear();
            limit = header.long_term_reference.then_some(0);
            current.long_term_index = limit;
        } else if header.adaptive_reference_marking {
            if header.memory_operations.len() > 64 {
                return Err(invalid("too many AVC memory operations"));
            }
            let mut reset = false;
            let mut marked_current = false;
            for op in &header.memory_operations {
                match *op {
                    MemoryOperation::ForgetShort(difference)
                    | MemoryOperation::ShortToLong { difference, .. } => {
                        if difference >= max {
                            return Err(invalid("AVC MMCO difference out of range"));
                        }
                        let target = i64::from(header.frame_num) - (i64::from(difference) + 1);
                        let index = frames
                            .iter()
                            .position(|(r, _, _, _)| {
                                r.long_term_index.is_none()
                                    && wrapped(r.frame_num, header.frame_num, max) == target
                            })
                            .ok_or_else(|| {
                                invalid("AVC MMCO selects missing short-term picture")
                            })?;
                        if let MemoryOperation::ShortToLong { index: long, .. } = *op {
                            if frames[index].1.is_none() {
                                return Err(invalid(
                                    "AVC MMCO cannot mark non-existing picture long-term",
                                ));
                            }
                            check_long(long, limit)?;
                            let target_id = frames[index].0.id;
                            frames.retain(|(r, _, _, _)| r.long_term_index != Some(long));
                            frames
                                .iter_mut()
                                .find(|(r, _, _, _)| r.id == target_id)
                                .unwrap()
                                .0
                                .long_term_index = Some(long);
                        } else {
                            frames.remove(index);
                        }
                    }
                    MemoryOperation::ForgetLong(long) => {
                        let index = frames
                            .iter()
                            .position(|(r, _, _, _)| r.long_term_index == Some(long))
                            .ok_or_else(|| invalid("AVC MMCO selects missing long-term picture"))?;
                        frames.remove(index);
                    }
                    MemoryOperation::LimitLong(plus_one) => {
                        if plus_one > self.capacity as u32 {
                            return Err(invalid("AVC long-term limit exceeds reference capacity"));
                        }
                        limit = plus_one.checked_sub(1);
                        frames.retain(|(r, _, _, _)| {
                            r.long_term_index
                                .is_none_or(|n| limit.is_some_and(|l| n <= l))
                        });
                    }
                    MemoryOperation::Reset => {
                        if reset || marked_current {
                            return Err(invalid("invalid AVC MMCO reset sequence"));
                        }
                        reset = true;
                        frames.clear();
                        limit = None;
                        current.frame_num = 0;
                    }
                    MemoryOperation::CurrentLong(long) => {
                        if marked_current || reset {
                            return Err(invalid("invalid AVC current long-term marking sequence"));
                        }
                        check_long(long, limit)?;
                        marked_current = true;
                        frames.retain(|(r, _, _, _)| r.long_term_index != Some(long));
                        current.long_term_index = Some(long);
                    }
                }
            }
        } else if frames.len() == self.capacity {
            let index = frames
                .iter()
                .enumerate()
                .filter(|(_, (r, _, _, _))| r.long_term_index.is_none())
                .min_by_key(|(_, (r, _, _, _))| wrapped(r.frame_num, header.frame_num, max))
                .map(|(i, _)| i)
                .ok_or_else(|| invalid("AVC sliding window has no short-term reference"))?;
            frames.remove(index);
        }
        if frames.len() >= self.capacity
            || frames.iter().any(|(r, _, _, _)| {
                r.id == id
                    || (r.long_term_index.is_none()
                        && current.long_term_index.is_none()
                        && r.frame_num == current.frame_num)
            })
        {
            return Err(invalid(
                "AVC reference buffer capacity or identity conflict",
            ));
        }
        frames.push((current, Some(picture), true, None));
        self.frames = frames;
        self.max_long_term_index = limit;
        self.initialized = true;
        Ok(())
    }
}
fn wrapped(frame_num: u32, current: u32, max: u32) -> i64 {
    i64::from(frame_num)
        - if frame_num > current {
            i64::from(max)
        } else {
            0
        }
}
fn check_long(index: u32, limit: Option<u32>) -> Result<()> {
    if !limit.is_some_and(|limit| index <= limit) {
        return Err(invalid("AVC long-term frame index exceeds limit"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::avc::{Pps, Sps};
    use super::*;
    fn header() -> SliceHeader {
        let hex = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let s = Sps::parse(&bytes).unwrap();
        let p = Pps::parse(&[0x68, 0xce, 0x09, 0xc8], &s).unwrap();
        SliceHeader::parse(&[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80], &s, &p).unwrap()
    }
    #[test]
    fn inferred_gap_entries_occupy_slots_without_sample_storage() {
        let mut buffer = ReferenceBuffer::new(4, 3).unwrap();
        let mut h = header();
        let shown = Arc::new(123);
        buffer.finish(&h, 0, 0, shown.clone()).unwrap();
        buffer.infer_nonexisting(1, None, 1).unwrap();
        buffer.infer_nonexisting(2, None, 2).unwrap();
        assert_eq!(
            buffer.references().iter().map(|r| r.id).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(buffer.get(1).is_none() && buffer.get(2).is_none());
        h.idr = false;
        h.frame_num = 3;
        h.slice_type = SliceType::P;
        h.refs_l0 = 1;
        assert_eq!(buffer.lists(&h, 2).unwrap().l0, [2]);
        h.modifications_l0 = vec![RefModification::Subtract(2)];
        assert_eq!(buffer.lists(&h, 2).unwrap().l0, [0]);
        buffer.finish(&h, 2, 3, Arc::new(456)).unwrap();
        assert_eq!(
            buffer.references().iter().map(|r| r.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(buffer.get(0).is_none());
        assert_eq!(*shown, 123); // output ownership outlives sliding eviction
    }
    #[test]
    fn b_lists_exclude_only_gap_entries_without_inferred_poc() {
        for known in [false, true] {
            let mut buffer = ReferenceBuffer::new(4, 4).unwrap();
            let mut h = header();
            buffer.finish(&h, 0, 0, Arc::new(0)).unwrap();
            buffer.infer_nonexisting(1, known.then_some(2), 1).unwrap();
            buffer.infer_nonexisting(2, known.then_some(4), 2).unwrap();
            h.idr = false;
            h.frame_num = 3;
            buffer.finish(&h, 8, 3, Arc::new(3)).unwrap();
            h.slice_type = SliceType::B;
            h.frame_num = 4;
            h.nal_ref_idc = 0;
            h.refs_l0 = 1;
            h.refs_l1 = 1;
            let lists = buffer.lists(&h, 3).unwrap();
            assert_eq!(lists.l0, if known { vec![1] } else { vec![0] });
            assert_eq!(lists.l1, if known { vec![2] } else { vec![3] });
            assert!(buffer.get(1).is_none() && buffer.get(2).is_none());
        }
    }
    #[test]
    fn gap_metadata_rejects_modification_and_long_term_assignment_but_allows_forgetting() {
        let mut buffer = ReferenceBuffer::new(4, 4).unwrap();
        let mut h = header();
        buffer.finish(&h, 0, 0, Arc::new(0)).unwrap();
        buffer.infer_nonexisting(1, None, 1).unwrap();
        h.idr = false;
        h.frame_num = 2;
        h.slice_type = SliceType::P;
        h.refs_l0 = 1;
        for command in [RefModification::Subtract(0), RefModification::Add(14)] {
            h.modifications_l0 = vec![command];
            assert!(
                buffer
                    .lists(&h, 4)
                    .unwrap_err()
                    .to_string()
                    .contains("non-existing")
            );
        }
        h.modifications_l0.clear();
        let before = buffer.references();
        h.adaptive_reference_marking = true;
        h.memory_operations = vec![
            MemoryOperation::LimitLong(1),
            MemoryOperation::ShortToLong {
                difference: 0,
                index: 0,
            },
        ];
        assert!(
            buffer
                .finish(&h, 4, 2, Arc::new(2))
                .unwrap_err()
                .to_string()
                .contains("non-existing")
        );
        assert_eq!(buffer.references(), before);
        assert_eq!(buffer.max_long_term_index, None);
        h.memory_operations = vec![MemoryOperation::ForgetShort(0)];
        buffer.finish(&h, 4, 2, Arc::new(2)).unwrap();
        assert_eq!(
            buffer.references().iter().map(|r| r.id).collect::<Vec<_>>(),
            [0, 2]
        );
    }
    #[test]
    fn inferred_sliding_window_wrap_and_failure_are_atomic() {
        let mut buffer = ReferenceBuffer::new(4, 2).unwrap();
        let mut h = header();
        buffer.finish(&h, 0, 0, Arc::new(0)).unwrap();
        h.idr = false;
        h.frame_num = 14;
        buffer.finish(&h, 28, 14, Arc::new(14)).unwrap();
        buffer.infer_nonexisting(15, Some(30), 15).unwrap();
        buffer.infer_nonexisting(0, Some(32), 16).unwrap();
        assert_eq!(
            buffer
                .references()
                .iter()
                .map(|r| r.frame_num)
                .collect::<Vec<_>>(),
            [15, 0]
        );
        let before = buffer.references();
        assert!(buffer.infer_nonexisting(16, None, 99).is_err());
        assert!(buffer.infer_nonexisting(1, None, 16).is_err());
        assert_eq!(buffer.references(), before);
        let mut uninitialized = ReferenceBuffer::<u8>::new(4, 2).unwrap();
        assert!(uninitialized.infer_nonexisting(1, None, 1).is_err());
        h = header();
        h.long_term_reference = true;
        let mut long = ReferenceBuffer::new(4, 1).unwrap();
        long.finish(&h, 0, 0, Arc::new(0)).unwrap();
        let before = long.references();
        assert!(long.infer_nonexisting(1, None, 1).is_err());
        assert_eq!(long.references(), before);
    }
    #[test]
    fn nonexisting_only_buffers_allow_intra_but_require_real_inter_references() {
        let mut buffer = ReferenceBuffer::new(4, 1).unwrap();
        let mut h = header();
        buffer.finish(&h, 0, 0, Arc::new(0)).unwrap();
        buffer.infer_nonexisting(1, None, 1).unwrap();
        h.idr = false;
        h.frame_num = 2;
        h.slice_type = SliceType::I;
        assert_eq!(buffer.lists(&h, 4).unwrap().l0, Vec::<u64>::new());
        h.slice_type = SliceType::P;
        h.refs_l0 = 1;
        assert!(
            buffer
                .lists(&h, 4)
                .unwrap_err()
                .to_string()
                .contains("no existing")
        );
        h.slice_type = SliceType::B;
        h.refs_l1 = 1;
        assert!(
            buffer
                .lists(&h, 4)
                .unwrap_err()
                .to_string()
                .contains("no existing")
        );
        h = header();
        buffer.finish(&h, 0, 2, Arc::new(2)).unwrap();
        assert_eq!(buffer.references().len(), 1);
        assert!(buffer.get(2).is_some());
    }
    #[test]
    fn sliding_window_wrap_and_retained_display_owner() {
        let mut b = ReferenceBuffer::new(4, 2).unwrap();
        let mut h = header();
        let displayed = Arc::new(42);
        b.finish(&h, 0, 0, displayed.clone()).unwrap();
        h.idr = false;
        h.frame_num = 14;
        b.finish(&h, 2, 1, Arc::new(43)).unwrap();
        h.frame_num = 15;
        b.finish(&h, 4, 2, Arc::new(44)).unwrap();
        assert!(b.get(0).is_none());
        h.frame_num = 0;
        b.finish(&h, 6, 3, Arc::new(45)).unwrap();
        assert!(b.get(1).is_none()); // frame 14 wraps to -2, older than frame 15.
        assert_eq!(*displayed, 42);
        h.nal_ref_idc = 0;
        b.finish(&h, 5, 9, Arc::new(46)).unwrap();
        assert!(b.get(9).is_none());
    }
    #[test]
    fn all_mmco_operations_and_atomic_failure() {
        let mut b = ReferenceBuffer::new(4, 3).unwrap();
        let mut h = header();
        b.finish(&h, 0, 0, Arc::new(0)).unwrap();
        h.idr = false;
        h.frame_num = 1;
        h.adaptive_reference_marking = true;
        h.memory_operations = vec![
            MemoryOperation::LimitLong(2),
            MemoryOperation::ShortToLong {
                difference: 0,
                index: 1,
            },
        ];
        b.finish(&h, 2, 1, Arc::new(1)).unwrap();
        assert_eq!(b.references()[0].long_term_index, Some(1));
        h.frame_num = 2;
        h.memory_operations = vec![
            MemoryOperation::ForgetShort(0),
            MemoryOperation::CurrentLong(1),
        ];
        b.finish(&h, 4, 2, Arc::new(2)).unwrap();
        assert_eq!(b.references().len(), 1);
        assert!(b.get(0).is_none());
        h.frame_num = 3;
        h.memory_operations = vec![
            MemoryOperation::LimitLong(0),
            MemoryOperation::ForgetLong(1),
        ];
        let before = b.references();
        assert!(b.finish(&h, 6, 3, Arc::new(3)).is_err());
        assert_eq!(before, b.references());
        h.memory_operations = vec![
            MemoryOperation::ForgetLong(1),
            MemoryOperation::CurrentLong(1),
        ];
        b.finish(&h, 6, 3, Arc::new(3)).unwrap(); // failed limit change did not commit.
        h.frame_num = 4;
        h.memory_operations = vec![MemoryOperation::Reset];
        b.finish(&h, 0, 4, Arc::new(4)).unwrap();
        assert_eq!(
            b.references(),
            [FrameReference {
                id: 4,
                frame_num: 0,
                poc: 0,
                long_term_index: None
            }]
        );
        h.frame_num = 1;
        h.memory_operations = vec![MemoryOperation::CurrentLong(0)];
        assert!(b.finish(&h, 2, 5, Arc::new(5)).is_err());
    }
    #[test]
    fn idr_flush_and_all_long_term_window_failure() {
        let mut b = ReferenceBuffer::new(4, 1).unwrap();
        let mut h = header();
        h.long_term_reference = true;
        b.finish(&h, 0, 0, Arc::new(0)).unwrap();
        h.idr = false;
        h.frame_num = 1;
        assert!(b.finish(&h, 2, 1, Arc::new(1)).is_err());
        assert!(b.get(0).is_some());
        h.idr = true;
        h.frame_num = 0;
        h.long_term_reference = false;
        b.finish(&h, 0, 2, Arc::new(2)).unwrap();
        assert!(b.get(0).is_none());
        assert!(b.get(2).is_some());
    }
    #[test]
    fn inferred_field_orders_preserve_asymmetric_pocs_and_atomic_validation() {
        let mut buffer = ReferenceBuffer::new(4, 3).unwrap();
        let mut header = header();
        header.idr = true;
        buffer.finish(&header, 0, 0, Arc::new(())).unwrap();
        let order = FieldOrder {
            top: Some(7),
            bottom: Some(3),
        };
        buffer.infer_nonexisting_fields(1, Some(order), 1).unwrap();
        assert_eq!(buffer.inferred_field_order(1), Some(order));
        assert!(buffer.get(1).is_none());
        assert_eq!(buffer.references()[1].poc, 3);
        let before = buffer.references();
        assert!(
            buffer
                .infer_nonexisting_fields(
                    2,
                    Some(FieldOrder {
                        top: Some(9),
                        bottom: None
                    }),
                    2
                )
                .is_err()
        );
        assert_eq!(buffer.references(), before);
        buffer.infer_nonexisting_fields(2, None, 2).unwrap();
        assert_eq!(buffer.inferred_field_order(2), None);
        buffer.infer_nonexisting_fields(3, Some(order), 3).unwrap();
        buffer.infer_nonexisting_fields(4, Some(order), 4).unwrap();
        assert_eq!(buffer.inferred_field_order(1), None);
    }
    #[test]
    fn migration_preserves_long_limits_gap_orders_and_reference_ids() {
        let mut h = header();
        h.long_term_reference = true;
        let mut frames = ReferenceBuffer::new(4, 4).unwrap();
        frames.finish(&h, 0, 50, Arc::new(7u8)).unwrap();
        frames.infer_nonexisting_fields(1, None, 51).unwrap();
        frames
            .infer_nonexisting_fields(
                2,
                Some(FieldOrder {
                    top: Some(4),
                    bottom: Some(4),
                }),
                52,
            )
            .unwrap();
        let mut fields = frames
            .into_fields(|r, p| {
                assert_eq!(r.id, 50);
                assert_eq!(r.long_term_index, Some(0));
                Ok((
                    [Arc::new(**p), Arc::new(**p + 1)],
                    FieldOrder {
                        top: Some(0),
                        bottom: Some(1),
                    },
                ))
            })
            .unwrap();
        assert_eq!(**fields.get(50, false).unwrap(), 7);
        assert_eq!(**fields.get(50, true).unwrap(), 8);
        assert_eq!(fields.order(50, false), Some((0, true)));
        assert_eq!(fields.order(50, true), Some((1, true)));
        for bottom in [false, true] {
            assert!(fields.get(51, bottom).is_none());
            assert!(fields.get(52, bottom).is_none());
            assert_eq!(fields.order(51, bottom), None);
            assert_eq!(fields.order(52, bottom), Some((4, false)));
        }
        h.idr = false;
        h.field_pic = true;
        h.bottom_field = false;
        h.frame_num = 3;
        h.slice_type = SliceType::P;
        h.refs_l0 = 1;
        h.modifications_l0 = vec![RefModification::LongTerm(1)];
        h.adaptive_reference_marking = true;
        h.memory_operations = vec![MemoryOperation::CurrentLong(1)];
        assert_eq!(fields.lists(&h, 6).unwrap().l0[0].id, 50);
        let before = fields.references();
        assert!(fields.finish(&h, 6, 53, Arc::new(9)).is_err());
        assert_eq!(fields.references(), before);
        h.memory_operations = vec![
            MemoryOperation::LimitLong(2),
            MemoryOperation::CurrentLong(1),
        ];
        fields.finish(&h, 6, 53, Arc::new(9)).unwrap();
        assert_eq!(fields.order(53, false), Some((6, true)));
    }
}
