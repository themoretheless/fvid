//! Progressive AVC reference-frame storage and marking (H.264 8.2.5).
//! Output/display reordering is separate; retain an `Arc` for pictures awaiting display.
use super::{
    avc_references::{FrameReference, ReferenceLists, frame_lists},
    avc_slice::{MemoryOperation, SliceHeader},
};
use crate::{Result, invalid};
use std::sync::Arc;

#[derive(Clone)]
pub struct ReferenceBuffer<T> {
    frame_num_bits: u8,
    capacity: usize,
    max_long_term_index: Option<u32>,
    initialized: bool,
    frames: Vec<(FrameReference, Arc<T>)>,
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
    pub fn get(&self, id: u64) -> Option<&Arc<T>> {
        self.frames.iter().find(|(r, _)| r.id == id).map(|(_, p)| p)
    }
    pub fn references(&self) -> Vec<FrameReference> {
        self.frames.iter().map(|(r, _)| *r).collect()
    }
    pub fn lists(&self, header: &SliceHeader, poc: i32) -> Result<ReferenceLists> {
        if header.field_pic {
            return Err(crate::unsupported("AVC field reference lists are not implemented"));
        }
        frame_lists(
            &self.references(),
            self.frame_num_bits,
            header.frame_num,
            poc,
            header.slice_type,
            [header.refs_l0 as usize, header.refs_l1 as usize],
            [&header.modifications_l0, &header.modifications_l1],
        )
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
            return Err(crate::unsupported("AVC field reference marking is not implemented"));
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
                            .position(|(r, _)| {
                                r.long_term_index.is_none()
                                    && wrapped(r.frame_num, header.frame_num, max) == target
                            })
                            .ok_or_else(|| {
                                invalid("AVC MMCO selects missing short-term picture")
                            })?;
                        if let MemoryOperation::ShortToLong { index: long, .. } = *op {
                            check_long(long, limit)?;
                            let target_id = frames[index].0.id;
                            frames.retain(|(r, _)| r.long_term_index != Some(long));
                            frames
                                .iter_mut()
                                .find(|(r, _)| r.id == target_id)
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
                            .position(|(r, _)| r.long_term_index == Some(long))
                            .ok_or_else(|| invalid("AVC MMCO selects missing long-term picture"))?;
                        frames.remove(index);
                    }
                    MemoryOperation::LimitLong(plus_one) => {
                        if plus_one > self.capacity as u32 {
                            return Err(invalid("AVC long-term limit exceeds reference capacity"));
                        }
                        limit = plus_one.checked_sub(1);
                        frames.retain(|(r, _)| {
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
                        frames.retain(|(r, _)| r.long_term_index != Some(long));
                        current.long_term_index = Some(long);
                    }
                }
            }
        } else if frames.len() == self.capacity {
            let index = frames
                .iter()
                .enumerate()
                .filter(|(_, (r, _))| r.long_term_index.is_none())
                .min_by_key(|(_, (r, _))| wrapped(r.frame_num, header.frame_num, max))
                .map(|(i, _)| i)
                .ok_or_else(|| invalid("AVC sliding window has no short-term reference"))?;
            frames.remove(index);
        }
        if frames.len() >= self.capacity
            || frames.iter().any(|(r, _)| {
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
        frames.push((current, picture));
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
}
