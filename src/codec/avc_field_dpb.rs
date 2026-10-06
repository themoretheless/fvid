//! Owned field DPB marking, pixel-free gap stores and parity-aware reference lists.
//! Mixed frame/field reference storage is handled separately.
use super::{
    avc_field_references::{FieldFrameReference, FieldLists, field_lists},
    avc_slice::{MemoryOperation, SliceHeader},
};
use crate::{Result, invalid};
use std::sync::Arc;
struct Field<T> {
    poc: i32,
    long: Option<u32>,
    picture: Option<Arc<T>>,
    known_poc: bool,
}
struct Store<T> {
    id: u64,
    frame_num: u32,
    fields: [Option<Field<T>>; 2],
}
pub struct FieldBuffer<T> {
    stores: Vec<Store<T>>,
    bits: u8,
    capacity: usize,
    limit: Option<u32>,
    initialized: bool,
    pending: Option<u64>,
}
impl<T> FieldBuffer<T> {
    pub fn new(bits: u8, capacity: u32) -> Result<Self> {
        if !(4..=16).contains(&bits) || capacity > 16 {
            return Err(invalid("invalid AVC field DPB configuration"));
        }
        Ok(Self {
            stores: Vec::new(),
            bits,
            capacity: capacity.max(1) as usize,
            limit: None,
            initialized: false,
            pending: None,
        })
    }
    pub(super) fn from_frame_storage<F>(
        bits: u8,
        capacity: usize,
        limit: Option<u32>,
        initialized: bool,
        frames: Vec<(
            super::avc_references::FrameReference,
            Option<Arc<F>>,
            bool,
            Option<super::avc_poc::FieldOrder>,
        )>,
        convert: impl Fn(
            super::avc_references::FrameReference,
            &Arc<F>,
        ) -> Result<([Arc<T>; 2], super::avc_poc::FieldOrder)>,
    ) -> Result<Self> {
        let mut result = Self::new(bits, capacity as u32)?;
        result.limit = limit;
        result.initialized = initialized;
        result
            .stores
            .try_reserve_exact(frames.len())
            .map_err(|_| invalid("cannot allocate AVC migrated field stores"))?;
        for (r, picture, known, inferred) in frames {
            let (pictures, order) = if let Some(p) = picture {
                let (pictures, order) = convert(r, &p)?;
                (
                    [
                        Some(Arc::clone(&pictures[0])),
                        Some(Arc::clone(&pictures[1])),
                    ],
                    Some(order),
                )
            } else {
                ([None, None], inferred)
            };
            if order.is_some_and(|o| o.top.is_none() || o.bottom.is_none()) {
                return Err(invalid("migrated AVC frame needs both field POCs"));
            }
            let fields = std::array::from_fn(|i| {
                Some(Field {
                    poc: order.map_or(r.poc, |o| {
                        if i == 0 {
                            o.top.unwrap()
                        } else {
                            o.bottom.unwrap()
                        }
                    }),
                    long: r.long_term_index,
                    picture: pictures[i].as_ref().map(Arc::clone),
                    known_poc: known,
                })
            });
            result.stores.push(Store {
                id: r.id,
                frame_num: r.frame_num,
                fields,
            });
        }
        Ok(result)
    }
    /// Transfer complete, equally marked pairs without changing decoded IDs.
    /// An incomplete or mixed-marked store needs unified field/frame storage;
    /// do not silently omit a live reference during this conversion.
    pub(super) fn into_frames<U>(
        self,
        convert: impl Fn(&Arc<T>, &Arc<T>, super::avc_poc::FieldOrder) -> Result<Arc<U>>,
    ) -> Result<super::avc_dpb::ReferenceBuffer<U>> {
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(self.stores.len())
            .map_err(|_| invalid("cannot allocate migrated AVC frames"))?;
        for store in self.stores {
            let [Some(top), Some(bottom)] = store.fields else {
                return Err(crate::unsupported(
                    "AVC incomplete field reference pair requires unified frame storage",
                ));
            };
            if top.long != bottom.long {
                return Err(crate::unsupported(
                    "AVC mixed field reference marking requires unified frame storage",
                ));
            }
            let order = super::avc_poc::FieldOrder {
                top: Some(top.poc),
                bottom: Some(bottom.poc),
            };
            let picture = match (&top.picture, &bottom.picture) {
                (Some(a), Some(b)) => Some(convert(a, b, order)?),
                (None, None) => None,
                _ => return Err(invalid("AVC migrated reference has only one decoded field")),
            };
            frames.push((
                super::avc_references::FrameReference {
                    id: store.id,
                    frame_num: store.frame_num,
                    poc: order.picture(),
                    long_term_index: top.long,
                },
                picture,
                top.known_poc && bottom.known_poc,
                (top.known_poc && bottom.known_poc).then_some(order),
            ));
        }
        super::avc_dpb::ReferenceBuffer::from_field_storage(
            self.bits,
            self.capacity,
            self.limit,
            self.initialized,
            frames,
        )
    }
    pub(super) fn has_frame_ineligible_stores(&self) -> bool {
        self.stores
            .iter()
            .any(|s| !matches!(&s.fields,[Some(a),Some(b)] if a.long==b.long))
    }
    /// Temporary frame list view. Non-paired/mixed fields stay in this owner;
    /// H.264 8.2.4.2.1 excludes non-paired fields. A complete mixed pair
    /// remains a short-term frame view, with independent field marking here.
    pub(super) fn frame_view<U>(
        &self,
        convert: impl Fn(&Arc<T>, &Arc<T>, super::avc_poc::FieldOrder) -> Result<Arc<U>>,
    ) -> Result<super::avc_dpb::ReferenceBuffer<U>> {
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(self.stores.len())
            .map_err(|_| invalid("cannot allocate AVC frame view"))?;
        for store in &self.stores {
            let [Some(top), Some(bottom)] = &store.fields else {
                continue;
            };
            if top.long.is_some() && bottom.long.is_some() && top.long != bottom.long {
                return Err(crate::unsupported(
                    "AVC mixed field reference marking requires unified frame storage",
                ));
            }
            let frame_long = top.long.filter(|_| bottom.long.is_some());
            let order = super::avc_poc::FieldOrder {
                top: Some(top.poc),
                bottom: Some(bottom.poc),
            };
            let picture = match (&top.picture, &bottom.picture) {
                (Some(a), Some(b)) => Some(convert(a, b, order)?),
                (None, None) => None,
                _ => return Err(invalid("AVC frame view has only one decoded field")),
            };
            let known = top.known_poc && bottom.known_poc;
            frames.push((
                super::avc_references::FrameReference {
                    id: store.id,
                    frame_num: store.frame_num,
                    poc: order.picture(),
                    long_term_index: frame_long,
                },
                picture,
                known,
                known.then_some(order),
            ));
        }
        super::avc_dpb::ReferenceBuffer::from_field_storage(
            self.bits,
            self.capacity,
            self.limit,
            self.initialized,
            frames,
        )
    }
    /// Frame marking over the canonical parity-aware stores (8.2.5).
    /// Temporary frame views must never independently evict retained fields.
    pub(super) fn finish_frame(
        &mut self,
        header: &SliceHeader,
        order: super::avc_poc::FieldOrder,
        id: u64,
        pictures: Option<[Arc<T>; 2]>,
    ) -> Result<()> {
        let max = 1u32 << self.bits;
        if header.field_pic
            || header.frame_num >= max
            || header.nal_ref_idc > 3
            || order.top.is_none()
            || order.bottom.is_none()
            || (header.idr && (header.frame_num != 0 || header.nal_ref_idc == 0))
        {
            return Err(invalid("invalid canonical AVC frame marking"));
        }
        if !header.idr && !self.initialized {
            return Err(invalid("canonical AVC frame DPB needs IDR"));
        }
        if header.nal_ref_idc == 0 {
            self.pending = None;
            return Ok(());
        }
        let pictures = pictures.ok_or_else(|| invalid("missing canonical AVC frame fields"))?;
        let mut stores: Vec<_> = self
            .stores
            .iter()
            .map(|s| Store {
                id: s.id,
                frame_num: s.frame_num,
                fields: std::array::from_fn(|i| {
                    s.fields[i].as_ref().map(|f| Field {
                        poc: f.poc,
                        long: f.long,
                        picture: f.picture.as_ref().map(Arc::clone),
                        known_poc: f.known_poc,
                    })
                }),
            })
            .collect();
        let mut limit = self.limit;
        let mut current_num = header.frame_num;
        let mut current_long = None;
        let wrap = |n: u32| {
            i64::from(n)
                - if n > header.frame_num {
                    i64::from(max)
                } else {
                    0
                }
        };
        let pair_short = |s: &Store<T>| matches!(&s.fields,[Some(a),Some(b)] if a.long.is_none()||b.long.is_none());
        let pair_long = |s: &Store<T>, index: u32| matches!(&s.fields,[Some(a),Some(b)] if a.long==Some(index)&&b.long==Some(index));
        let check = |index: u32, limit: Option<u32>| {
            if limit.is_some_and(|l| index <= l) {
                Ok(())
            } else {
                Err(invalid("canonical AVC long-term index exceeds limit"))
            }
        };
        if header.idr {
            stores.clear();
            limit = header.long_term_reference.then_some(0);
            current_long = limit;
        } else if header.adaptive_reference_marking {
            if header.memory_operations.len() > 64 {
                return Err(invalid("too many canonical AVC frame memory operations"));
            }
            let mut reset = false;
            let mut marked = false;
            for op in &header.memory_operations {
                match *op {
                    MemoryOperation::ForgetShort(difference)
                    | MemoryOperation::ShortToLong { difference, .. } => {
                        if difference >= max {
                            return Err(invalid(
                                "canonical AVC frame MMCO difference out of range",
                            ));
                        }
                        let target = i64::from(header.frame_num) - i64::from(difference) - 1;
                        let index = stores
                            .iter()
                            .position(|s| pair_short(s) && wrap(s.frame_num) == target)
                            .ok_or_else(|| {
                                invalid("canonical AVC frame MMCO selects missing short pair")
                            })?;
                        if let MemoryOperation::ShortToLong { index: long, .. } = *op {
                            check(long, limit)?;
                            if stores[index]
                                .fields
                                .iter()
                                .flatten()
                                .any(|f| f.picture.is_none())
                            {
                                return Err(invalid(
                                    "canonical AVC non-existing pair cannot become long-term",
                                ));
                            }
                            let selected = stores[index].id;
                            for s in &mut stores {
                                if s.id != selected {
                                    for f in &mut s.fields {
                                        if f.as_ref().is_some_and(|f| f.long == Some(long)) {
                                            *f = None;
                                        }
                                    }
                                }
                            }
                            for f in stores[index].fields.iter_mut().flatten() {
                                f.long = Some(long);
                            }
                        } else {
                            stores[index].fields = [None, None];
                        }
                    }
                    MemoryOperation::ForgetLong(long) => {
                        let index =
                            stores
                                .iter()
                                .position(|s| pair_long(s, long))
                                .ok_or_else(|| {
                                    invalid("canonical AVC frame MMCO selects missing long pair")
                                })?;
                        stores[index].fields = [None, None];
                    }
                    MemoryOperation::LimitLong(plus_one) => {
                        if plus_one > self.capacity as u32 {
                            return Err(invalid("canonical AVC long-term limit exceeds capacity"));
                        }
                        limit = plus_one.checked_sub(1);
                        for s in &mut stores {
                            for f in &mut s.fields {
                                if f.as_ref().is_some_and(|f| {
                                    f.long.is_some_and(|n| limit.is_none_or(|l| n > l))
                                }) {
                                    *f = None;
                                }
                            }
                        }
                    }
                    MemoryOperation::Reset => {
                        if reset || marked {
                            return Err(invalid("invalid canonical AVC frame reset sequence"));
                        }
                        reset = true;
                        stores.clear();
                        limit = None;
                        current_num = 0;
                    }
                    MemoryOperation::CurrentLong(long) => {
                        if marked || reset {
                            return Err(invalid("invalid canonical AVC current-long sequence"));
                        }
                        check(long, limit)?;
                        marked = true;
                        current_long = Some(long);
                        for s in &mut stores {
                            for f in &mut s.fields {
                                if f.as_ref().is_some_and(|f| f.long == Some(long)) {
                                    *f = None;
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // A mixed-marked store contributes once to each count. Sliding
            // A complete pair eviction removes both fields; a non-paired
            // long field is preserved when a separate short field is removed.
            let count: usize = stores
                .iter()
                .map(|s| {
                    usize::from(s.fields.iter().flatten().any(|f| f.long.is_none()))
                        + usize::from(s.fields.iter().flatten().any(|f| f.long.is_some()))
                })
                .sum();
            if count >= self.capacity {
                let oldest = stores
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.fields.iter().flatten().any(|f| f.long.is_none()))
                    .min_by_key(|(_, s)| wrap(s.frame_num))
                    .map(|(i, _)| i)
                    .ok_or_else(|| {
                        invalid("canonical AVC sliding window has no short reference")
                    })?;
                if stores[oldest].fields.iter().all(Option::is_some) {
                    stores[oldest].fields = [None, None];
                } else {
                    for f in &mut stores[oldest].fields {
                        if f.as_ref().is_some_and(|f| f.long.is_none()) {
                            *f = None;
                        }
                    }
                }
            }
        }
        stores.retain(|s| s.fields.iter().any(Option::is_some));
        if stores.len() >= self.capacity
            || stores.iter().any(|s| {
                s.id == id
                    || (s.frame_num == current_num
                        && current_long.is_none()
                        && s.fields.iter().flatten().any(|f| f.long.is_none()))
            })
        {
            return Err(invalid("canonical AVC frame capacity or identity conflict"));
        }
        stores.push(Store {
            id,
            frame_num: current_num,
            fields: std::array::from_fn(|p| {
                Some(Field {
                    poc: if p == 0 {
                        order.top.unwrap()
                    } else {
                        order.bottom.unwrap()
                    },
                    long: current_long,
                    picture: Some(Arc::clone(&pictures[p])),
                    known_poc: true,
                })
            }),
        });
        self.stores = stores;
        self.limit = limit;
        self.pending = None;
        self.initialized = true;
        Ok(())
    }
    pub fn references(&self) -> Vec<FieldFrameReference> {
        self.stores
            .iter()
            .map(|s| FieldFrameReference {
                id: s.id,
                frame_num: s.frame_num,
                top_poc: s.fields[0].as_ref().map(|f| f.poc),
                bottom_poc: s.fields[1].as_ref().map(|f| f.poc),
                long_term_indices: std::array::from_fn(|i| {
                    s.fields[i].as_ref().and_then(|f| f.long)
                }),
            })
            .collect()
    }
    pub fn get(&self, id: u64, bottom: bool) -> Option<&Arc<T>> {
        self.stores.iter().find(|s| s.id == id)?.fields[usize::from(bottom)]
            .as_ref()
            .and_then(|f| f.picture.as_ref())
    }
    /// POC and long-term status of the selected field, in its own parity.
    pub fn order(&self, id: u64, bottom: bool) -> Option<(i32, bool)> {
        let field = self.stores.iter().find(|store| store.id == id)?.fields[usize::from(bottom)]
            .as_ref()?;
        field.known_poc.then_some((field.poc, field.long.is_some()))
    }
    pub fn lists(&self, header: &SliceHeader, poc: i32) -> Result<FieldLists> {
        if !header.field_pic {
            return Err(invalid("field DPB requires field slice"));
        }
        let references = self
            .references()
            .into_iter()
            .filter(|r| {
                header.slice_type != super::avc_slice::SliceType::B
                    || self
                        .stores
                        .iter()
                        .find(|s| s.id == r.id)
                        .unwrap()
                        .fields
                        .iter()
                        .flatten()
                        .all(|f| f.known_poc)
            })
            .collect::<Vec<_>>();
        field_lists(
            &references,
            self.bits,
            header.frame_num,
            poc,
            header.bottom_field,
            header.slice_type,
            [header.refs_l0 as usize, header.refs_l1 as usize],
            [&header.modifications_l0, &header.modifications_l1],
        )
    }
    /// Insert a pixel-free non-existing frame store for a frame_num gap.
    /// Missing POC (type0) participates in P list order, but not B POC order.
    pub fn infer_nonexisting_fields(
        &mut self,
        frame_num: u32,
        poc: Option<super::avc_poc::FieldOrder>,
        id: u64,
    ) -> Result<()> {
        if poc.is_some_and(|p| p.top.is_none() || p.bottom.is_none()) {
            return Err(invalid("AVC inferred frame requires both field POCs"));
        }
        let max = 1u32 << self.bits;
        if !self.initialized || frame_num >= max || self.pending.is_some() {
            return Err(invalid(
                "AVC inferred fields need an initialized complete reference pair",
            ));
        }
        let mut stores = self
            .stores
            .iter()
            .map(|s| Store {
                id: s.id,
                frame_num: s.frame_num,
                fields: std::array::from_fn(|i| {
                    s.fields[i].as_ref().map(|f| Field {
                        poc: f.poc,
                        long: f.long,
                        picture: f.picture.as_ref().map(Arc::clone),
                        known_poc: f.known_poc,
                    })
                }),
            })
            .collect::<Vec<_>>();
        if stores.len() == self.capacity {
            let oldest = stores
                .iter()
                .enumerate()
                .filter(|(_, s)| s.fields.iter().flatten().any(|f| f.long.is_none()))
                .min_by_key(|(_, s)| {
                    i64::from(s.frame_num)
                        - if s.frame_num > frame_num {
                            i64::from(max)
                        } else {
                            0
                        }
                })
                .map(|(i, _)| i)
                .ok_or_else(|| invalid("AVC field sliding window has no short reference"))?;
            for f in &mut stores[oldest].fields {
                if f.as_ref().is_some_and(|f| f.long.is_none()) {
                    *f = None;
                }
            }
            stores.retain(|s| s.fields.iter().any(Option::is_some));
        }
        if stores.len() >= self.capacity
            || stores.iter().any(|s| {
                s.id == id
                    || (s.frame_num == frame_num
                        && s.fields.iter().flatten().any(|f| f.long.is_none()))
            })
        {
            return Err(invalid("AVC inferred field capacity or identity conflict"));
        }
        let fields = std::array::from_fn(|i| {
            Some(Field {
                poc: poc.map_or(0, |p| {
                    if i == 0 {
                        p.top.unwrap_or(0)
                    } else {
                        p.bottom.unwrap_or(0)
                    }
                }),
                long: None,
                picture: None,
                known_poc: poc.is_some(),
            })
        });
        stores.push(Store {
            id,
            frame_num,
            fields,
        });
        self.stores = stores;
        Ok(())
    }
    /// Mark a reconstructed field. Errors leave all stored references/limits unchanged.
    /// `poc` is post-MMCO-5 POC; caller owns display pairing and sample budgets.
    pub fn finish(
        &mut self,
        header: &SliceHeader,
        poc: i32,
        id: u64,
        picture: Arc<T>,
    ) -> Result<()> {
        let max = 1u32 << self.bits;
        let parity = usize::from(header.bottom_field);
        if !header.field_pic
            || header.frame_num >= max
            || header.nal_ref_idc > 3
            || (header.idr && (header.frame_num != 0 || header.nal_ref_idc == 0))
        {
            return Err(invalid("invalid AVC field marking header"));
        }
        if !header.idr && !self.initialized {
            return Err(invalid("AVC field DPB needs IDR"));
        }
        if header.nal_ref_idc == 0 {
            self.pending = None;
            return Ok(());
        }
        let mut stores: Vec<_> = self
            .stores
            .iter()
            .map(|s| Store {
                id: s.id,
                frame_num: s.frame_num,
                fields: std::array::from_fn(|i| {
                    s.fields[i].as_ref().map(|f| Field {
                        poc: f.poc,
                        long: f.long,
                        picture: f.picture.as_ref().map(Arc::clone),
                        known_poc: f.known_poc,
                    })
                }),
            })
            .collect();
        let mut limit = self.limit;
        let mut current_num = header.frame_num;
        let mut current_long = None;
        let wrap = |n: u32| {
            i64::from(n)
                - if n > header.frame_num {
                    i64::from(max)
                } else {
                    0
                }
        };
        let check_long = |index: u32, limit: Option<u32>| -> Result<()> {
            if !limit.is_some_and(|n| index <= n) {
                Err(invalid("AVC field long-term index exceeds limit"))
            } else {
                Ok(())
            }
        };
        if header.idr {
            stores.clear();
            limit = header.long_term_reference.then_some(0);
            current_long = limit;
        } else if header.adaptive_reference_marking {
            if header.memory_operations.len() > 64 {
                return Err(invalid("too many AVC field memory operations"));
            }
            let mut reset = false;
            let mut marked = false;
            for op in &header.memory_operations {
                match *op {
                    MemoryOperation::ForgetShort(difference)
                    | MemoryOperation::ShortToLong { difference, .. } => {
                        if difference >= 2 * max {
                            return Err(invalid("AVC field MMCO difference out of range"));
                        }
                        let target =
                            2 * i64::from(header.frame_num) + 1 - i64::from(difference) - 1;
                        let selected = stores
                            .iter()
                            .enumerate()
                            .find_map(|(i, s)| {
                                (0..2)
                                    .find(|p| {
                                        s.fields[*p].as_ref().is_some_and(|f| f.long.is_none())
                                            && 2 * wrap(s.frame_num) + i64::from(*p == parity)
                                                == target
                                    })
                                    .map(|p| (i, p))
                            })
                            .ok_or_else(|| invalid("AVC field MMCO selects missing short field"))?;
                        if let MemoryOperation::ShortToLong { index, .. } = *op {
                            check_long(index, limit)?;
                            if stores[selected.0].fields[selected.1]
                                .as_ref()
                                .unwrap()
                                .picture
                                .is_none()
                            {
                                return Err(invalid(
                                    "AVC non-existing field cannot become long-term",
                                ));
                            }
                            let target_id = stores[selected.0].id;
                            for s in &mut stores {
                                for p in 0..2 {
                                    if s.fields[p].as_ref().is_some_and(|f| f.long == Some(index))
                                        && (s.id != target_id || p == selected.1)
                                    {
                                        s.fields[p] = None;
                                    }
                                }
                            }
                            stores[selected.0].fields[selected.1].as_mut().unwrap().long =
                                Some(index);
                        } else {
                            stores[selected.0].fields[selected.1] = None;
                        }
                    }
                    MemoryOperation::ForgetLong(number) => {
                        let selected = stores
                            .iter()
                            .enumerate()
                            .find_map(|(i, s)| {
                                (0..2)
                                    .find(|p| {
                                        s.fields[*p].as_ref().is_some_and(|f| {
                                            f.long.is_some_and(|n| {
                                                2 * n + u32::from(*p == parity) == number
                                            })
                                        })
                                    })
                                    .map(|p| (i, p))
                            })
                            .ok_or_else(|| invalid("AVC field MMCO selects missing long field"))?;
                        stores[selected.0].fields[selected.1] = None;
                    }
                    MemoryOperation::LimitLong(plus_one) => {
                        if plus_one > self.capacity as u32 {
                            return Err(invalid("AVC field long-term limit exceeds capacity"));
                        }
                        limit = plus_one.checked_sub(1);
                        for s in &mut stores {
                            for f in &mut s.fields {
                                if f.as_ref().is_some_and(|f| {
                                    f.long.is_some_and(|n| limit.is_none_or(|l| n > l))
                                }) {
                                    *f = None;
                                }
                            }
                        }
                    }
                    MemoryOperation::Reset => {
                        if reset || marked {
                            return Err(invalid("invalid AVC field MMCO reset sequence"));
                        }
                        reset = true;
                        stores.clear();
                        limit = None;
                        current_num = 0;
                    }
                    MemoryOperation::CurrentLong(index) => {
                        if marked || reset {
                            return Err(invalid("invalid AVC field current long-term sequence"));
                        }
                        check_long(index, limit)?;
                        marked = true;
                        current_long = Some(index);
                        for s in &mut stores {
                            for p in 0..2 {
                                if s.fields[p].as_ref().is_some_and(|f| f.long == Some(index))
                                    && (s.frame_num != current_num || p == parity)
                                {
                                    s.fields[p] = None;
                                }
                            }
                        }
                    }
                }
            }
        }
        stores.retain(|s| s.fields.iter().any(Option::is_some));
        let complement = stores.iter().position(|s| {
            self.pending == Some(s.id)
                && s.frame_num == current_num
                && s.fields[parity].is_none()
                && s.fields[1 - parity].is_some()
        });
        if complement.is_none()
            && !header.adaptive_reference_marking
            && !header.idr
            && stores.len() == self.capacity
        {
            let oldest = stores
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.fields
                        .iter()
                        .any(|f| f.as_ref().is_some_and(|f| f.long.is_none()))
                })
                .min_by_key(|(_, s)| wrap(s.frame_num))
                .map(|(i, _)| i)
                .ok_or_else(|| invalid("AVC field sliding window has no short reference"))?;
            for field in &mut stores[oldest].fields {
                if field.as_ref().is_some_and(|f| f.long.is_none()) {
                    *field = None;
                }
            }
            stores.retain(|s| s.fields.iter().any(Option::is_some));
        }
        if stores.iter().any(|s| s.id == id) {
            return Err(invalid("duplicate AVC field storage ID"));
        }
        let field = Field {
            poc,
            long: current_long,
            picture: Some(picture),
            known_poc: true,
        };
        if let Some(index) = complement {
            stores[index].fields[parity] = Some(field);
        } else {
            if stores.len() >= self.capacity
                || stores.iter().any(|s| {
                    s.frame_num == current_num
                        && current_long.is_none()
                        && s.fields[parity].as_ref().is_some_and(|f| f.long.is_none())
                })
            {
                return Err(invalid("AVC field DPB capacity or identity conflict"));
            }
            let mut fields = [None, None];
            fields[parity] = Some(field);
            stores.push(Store {
                id,
                frame_num: current_num,
                fields,
            });
        }
        // Validate field identity/long-term uniqueness before committing.
        let refs: Vec<_> = stores
            .iter()
            .map(|s| FieldFrameReference {
                id: s.id,
                frame_num: s.frame_num,
                top_poc: s.fields[0].as_ref().map(|f| f.poc),
                bottom_poc: s.fields[1].as_ref().map(|f| f.poc),
                long_term_indices: std::array::from_fn(|i| {
                    s.fields[i].as_ref().and_then(|f| f.long)
                }),
            })
            .collect();
        field_lists(
            &refs,
            self.bits,
            current_num,
            poc,
            header.bottom_field,
            super::avc_slice::SliceType::I,
            [0, 0],
            [&[], &[]],
        )?;
        self.pending = if complement.is_some() { None } else { Some(id) };
        self.stores = stores;
        self.limit = limit;
        self.initialized = true;
        Ok(())
    }
}

#[cfg(test)]
mod migration_tests {
    use super::super::avc_poc::FieldOrder;
    use super::*;
    #[test]
    fn canonical_frame_mmco_matches_frame_buffer_for_complete_pairs() {
        use super::super::{
            avc::Pps,
            avc::Sps,
            avc_dpb::ReferenceBuffer,
            config::{AvcConfig, NalUnits},
        };
        use crate::container::mp4::Mp4Reader;
        use std::io::Cursor;
        let video = include_bytes!(
            "../../tests/fixtures/playback-errors/avc-frame-field-direct-paff-8bit-top-temporal-coded-motion.mp4"
        );
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let config = input.tracks()[0].configuration.clone();
        let avc = AvcConfig::parse(&config).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        let mut intra_packet = Vec::new();
        input.read_packet(0, 0, &mut intra_packet).unwrap();
        let mut inter_packet = Vec::new();
        input.read_packet(0, 1, &mut inter_packet).unwrap();
        let parse = |is_intra| {
            SliceHeader::parse(
                NalUnits::new(
                    if is_intra {
                        &intra_packet
                    } else {
                        &inter_packet
                    },
                    avc.length_size,
                )
                .unwrap()
                .next()
                .unwrap()
                .unwrap(),
                &sps,
                &pps,
            )
            .unwrap()
        };
        for initial_long in [false, true] {
            for operations in [
                vec![],
                vec![MemoryOperation::ForgetShort(0)],
                vec![MemoryOperation::ForgetLong(0)],
                vec![MemoryOperation::LimitLong(0)],
                vec![
                    MemoryOperation::LimitLong(1),
                    MemoryOperation::ShortToLong {
                        difference: 1,
                        index: 0,
                    },
                    MemoryOperation::ForgetShort(0),
                ],
                vec![
                    MemoryOperation::LimitLong(1),
                    MemoryOperation::CurrentLong(0),
                    MemoryOperation::ForgetShort(0),
                ],
                vec![MemoryOperation::Reset],
                vec![MemoryOperation::ForgetShort(15)],
                vec![MemoryOperation::Reset, MemoryOperation::CurrentLong(0)],
            ] {
                let mut frames = ReferenceBuffer::<u8>::new(4, 3).unwrap();
                let mut fields = FieldBuffer::<u8>::new(4, 3).unwrap();
                for number in 0..3u32 {
                    let mut h = parse(number == 0);
                    h.frame_num = number;
                    h.long_term_reference = number == 0 && initial_long;
                    let order = FieldOrder {
                        top: Some((number * 2) as i32),
                        bottom: Some((number * 2 + 1) as i32),
                    };
                    frames
                        .finish(
                            &h,
                            order.picture(),
                            u64::from(number),
                            Arc::new(number as u8),
                        )
                        .unwrap();
                    fields
                        .finish_frame(
                            &h,
                            order,
                            u64::from(number),
                            Some([Arc::new(number as u8), Arc::new(number as u8)]),
                        )
                        .unwrap();
                }
                let before = fields.references();
                let mut h = parse(false);
                h.frame_num = 3;
                h.adaptive_reference_marking = !operations.is_empty();
                let label = format!("{initial_long} {operations:?}");
                let reset = operations.contains(&MemoryOperation::Reset);
                h.memory_operations = operations;
                let order = if reset {
                    FieldOrder {
                        top: Some(0),
                        bottom: Some(1),
                    }
                } else {
                    FieldOrder {
                        top: Some(6),
                        bottom: Some(7),
                    }
                };
                let a = frames.finish(&h, order.picture(), 3, Arc::new(3));
                let b = fields.finish_frame(&h, order, 3, Some([Arc::new(3), Arc::new(3)]));
                assert_eq!(a.is_ok(), b.is_ok(), "{label}");
                if b.is_err() {
                    assert_eq!(fields.references(), before);
                }
                let view = fields.frame_view(|a, _, _| Ok(Arc::clone(a))).unwrap();
                assert_eq!(frames.references(), view.references(), "{label}");
            }
        }
    }
    #[test]
    fn complete_pairs_transfer_ids_marking_pocs_and_pixel_free_gaps() {
        let mut fields = FieldBuffer::new(4, 3).unwrap();
        fields.limit = Some(2);
        fields.initialized = true;
        fields.stores = vec![
            Store {
                id: 7,
                frame_num: 15,
                fields: [
                    Some(Field {
                        poc: 8,
                        long: Some(2),
                        picture: Some(Arc::new(10)),
                        known_poc: true,
                    }),
                    Some(Field {
                        poc: 4,
                        long: Some(2),
                        picture: Some(Arc::new(20)),
                        known_poc: true,
                    }),
                ],
            },
            Store {
                id: 8,
                frame_num: 0,
                fields: [
                    Some(Field {
                        poc: 10,
                        long: None,
                        picture: None,
                        known_poc: false,
                    }),
                    Some(Field {
                        poc: 11,
                        long: None,
                        picture: None,
                        known_poc: false,
                    }),
                ],
            },
        ];
        let old = Arc::downgrade(fields.get(7, false).unwrap());
        let frames = fields
            .into_frames(|a, b, order| {
                assert_eq!(
                    order,
                    FieldOrder {
                        top: Some(8),
                        bottom: Some(4)
                    }
                );
                Ok(Arc::new(**a + **b))
            })
            .unwrap();
        assert!(old.upgrade().is_none());
        assert_eq!(**frames.get(7).unwrap(), 30);
        assert!(frames.get(8).is_none());
        assert_eq!(
            frames.references(),
            vec![
                super::super::avc_references::FrameReference {
                    id: 7,
                    frame_num: 15,
                    poc: 4,
                    long_term_index: Some(2)
                },
                super::super::avc_references::FrameReference {
                    id: 8,
                    frame_num: 0,
                    poc: 10,
                    long_term_index: None
                },
            ]
        );
        assert_eq!(frames.inferred_field_order(8), None);
        let restored = frames
            .into_fields(|_, a| {
                Ok((
                    [Arc::clone(a), Arc::clone(a)],
                    FieldOrder {
                        top: Some(8),
                        bottom: Some(4),
                    },
                ))
            })
            .unwrap();
        assert_eq!(restored.limit, Some(2));
        assert!(restored.initialized);
        assert!(restored.pending.is_none());
        for p in 0..2 {
            assert!(!restored.stores[1].fields[p].as_ref().unwrap().known_poc);
            assert_eq!(
                restored.order(7, p != 0),
                Some((if p == 0 { 8 } else { 4 }, true))
            );
        }
    }
    #[test]
    fn unsupported_pair_shapes_are_not_silently_discarded() {
        for mixed in [false, true] {
            let mut fields = FieldBuffer::new(4, 3).unwrap();
            fields.initialized = true;
            fields.stores.push(Store {
                id: 1,
                frame_num: 0,
                fields: [
                    Some(Field {
                        poc: 0,
                        long: None,
                        picture: Some(Arc::new(1)),
                        known_poc: true,
                    }),
                    mixed.then(|| Field {
                        poc: 1,
                        long: Some(1),
                        picture: Some(Arc::new(2)),
                        known_poc: true,
                    }),
                ],
            });
            let error = fields
                .into_frames::<i32>(|_, _, _| {
                    panic!("unsupported metadata must fail before conversion")
                })
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains(if mixed {
                    "mixed field reference marking"
                } else {
                    "incomplete field reference pair"
                }),
                "{error}"
            );
        }
    }
}
