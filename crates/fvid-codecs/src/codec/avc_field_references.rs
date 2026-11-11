//! Field-list foundation (H.264 8.2.4). DPB field marking/integration is separate.
use super::avc_slice::{RefModification, SliceType};
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldFrameReference {
    pub id: u64,
    pub frame_num: u32,
    pub top_poc: Option<i32>,
    pub bottom_poc: Option<i32>,
    /// Independent marking for top (0) and bottom (1); None is short-term.
    pub long_term_indices: [Option<u32>; 2],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSelection {
    pub id: u64,
    pub bottom: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub struct FieldLists {
    pub l0: Vec<FieldSelection>,
    pub l1: Vec<FieldSelection>,
}
impl FieldFrameReference {
    fn available(self, bottom: bool) -> bool {
        if bottom {
            self.bottom_poc.is_some()
        } else {
            self.top_poc.is_some()
        }
    }
    fn poc(self) -> i32 {
        match (self.top_poc, self.bottom_poc) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => unreachable!("validated field frame"),
        }
    }
}
fn expand(frames: &[FieldFrameReference], bottom: bool) -> Vec<FieldSelection> {
    let mut cursors = [0usize; 2];
    let mut result = Vec::with_capacity(frames.len() * 2);
    loop {
        let mut added = false;
        for (cursor, parity) in cursors.iter_mut().zip([bottom, !bottom]) {
            while *cursor < frames.len() {
                let frame = frames[*cursor];
                *cursor += 1;
                if frame.available(parity) && frame.long_term_indices[usize::from(parity)].is_none()
                {
                    result.push(FieldSelection {
                        id: frame.id,
                        bottom: parity,
                    });
                    added = true;
                    break;
                }
            }
        }
        if !added {
            break;
        }
    }
    result
}
fn expand_long(frames: &[FieldFrameReference], bottom: bool) -> Vec<FieldSelection> {
    let mut parities = [Vec::new(), Vec::new()];
    for parity in [false, true] {
        for frame in frames {
            if let Some(index) = frame.long_term_indices[usize::from(parity)] {
                if frame.available(parity) {
                    parities[usize::from(parity)].push((
                        index,
                        FieldSelection {
                            id: frame.id,
                            bottom: parity,
                        },
                    ));
                }
            }
        }
        parities[usize::from(parity)].sort_by_key(|v| v.0);
    }
    let mut result = Vec::with_capacity(frames.len() * 2);
    for index in 0..parities[0].len().max(parities[1].len()) {
        for parity in [bottom, !bottom] {
            if let Some((_, selection)) = parities[usize::from(parity)].get(index) {
                result.push(*selection);
            }
        }
    }
    result
}
/// Build lists from complete or unpaired stores with independent field marking.
/// The caller excludes the current field and unavailable slots.
pub fn field_lists(
    references: &[FieldFrameReference],
    bits: u8,
    frame_num: u32,
    poc: i32,
    bottom: bool,
    kind: SliceType,
    active: [usize; 2],
    commands: [&[RefModification]; 2],
) -> Result<FieldLists> {
    if !(4..=16).contains(&bits) || references.len() > 16 {
        return Err(invalid("invalid AVC field reference configuration"));
    }
    let max = 1u32 << bits;
    if frame_num >= max || active.iter().any(|v| *v > 32) {
        return Err(invalid("invalid AVC field reference count or frame number"));
    }
    for (index, r) in references.iter().enumerate() {
        if r.frame_num >= max
            || (r.top_poc.is_none() && r.bottom_poc.is_none())
            || r.long_term_indices
                .iter()
                .any(|n| n.is_some_and(|v| v >= 16))
        {
            return Err(invalid("invalid AVC reference field store"));
        }
        for parity in [false, true] {
            let slot = usize::from(parity);
            if !r.available(parity) && r.long_term_indices[slot].is_some() {
                return Err(invalid("missing AVC field has long-term marking"));
            }
            if references[..index].iter().any(|p| {
                p.id == r.id
                    || (r.available(parity)
                        && p.available(parity)
                        && match (p.long_term_indices[slot], r.long_term_indices[slot]) {
                            (None, None) => p.frame_num == r.frame_num,
                            (Some(a), Some(b)) => a == b,
                            _ => false,
                        })
            }) {
                return Err(invalid("duplicate AVC reference field store"));
            }
        }
    }
    if matches!(kind, SliceType::I | SliceType::Si) {
        return Ok(FieldLists {
            l0: Vec::new(),
            l1: Vec::new(),
        });
    }
    let wrap = |r: &FieldFrameReference| {
        i64::from(r.frame_num)
            - if r.frame_num > frame_num {
                i64::from(max)
            } else {
                0
            }
    };
    let mut short: Vec<_> = references
        .iter()
        .filter(|r| {
            [false, true]
                .into_iter()
                .any(|p| r.available(p) && r.long_term_indices[usize::from(p)].is_none())
        })
        .copied()
        .collect();
    let long = expand_long(references, bottom);
    let mut l1 = Vec::new();
    if kind == SliceType::B {
        short.sort_by_key(|r| {
            if r.poc() <= poc {
                (0, -i64::from(r.poc()))
            } else {
                (1, i64::from(r.poc()))
            }
        });
        let mut future = short.clone();
        future.sort_by_key(|r| {
            if r.poc() > poc {
                (0, i64::from(r.poc()))
            } else {
                (1, -i64::from(r.poc()))
            }
        });
        l1 = expand(&future, bottom);
        l1.extend_from_slice(&long);
    } else {
        short.sort_by_key(|r| -wrap(r));
    }
    let mut l0 = expand(&short, bottom);
    l0.extend_from_slice(&long);
    if l1.len() > 1 && l1 == l0 {
        l1.swap(0, 1);
    }
    let current = 2 * i64::from(frame_num) + 1;
    let max_pic = 2 * i64::from(max);
    let modify = |mut list: Vec<FieldSelection>,
                  count: usize,
                  mods: &[RefModification]|
     -> Result<Vec<FieldSelection>> {
        if count == 0 || mods.len() > count {
            return Err(invalid("invalid AVC active field reference count"));
        }
        let mut predicted = current;
        for (position, command) in mods.iter().enumerate() {
            let target = match *command {
                RefModification::LongTerm(n) => i64::from(n),
                RefModification::Subtract(n) | RefModification::Add(n) => {
                    if i64::from(n) >= max_pic {
                        return Err(invalid("AVC field modification difference out of range"));
                    }
                    let delta = i64::from(n) + 1;
                    predicted = (predicted
                        + if matches!(command, RefModification::Subtract(_)) {
                            -delta
                        } else {
                            delta
                        })
                    .rem_euclid(max_pic);
                    predicted - if predicted > current { max_pic } else { 0 }
                }
            };
            let mut selected = None;
            for r in references {
                for parity in [false, true] {
                    if !r.available(parity) {
                        continue;
                    }
                    let matches = match command {
                        RefModification::LongTerm(_) => r.long_term_indices[usize::from(parity)]
                            .is_some_and(|n| {
                                2 * i64::from(n) + i64::from(parity == bottom) == target
                            }),
                        _ => {
                            r.long_term_indices[usize::from(parity)].is_none()
                                && 2 * wrap(r) + i64::from(parity == bottom) == target
                        }
                    };
                    if matches {
                        selected = Some(FieldSelection {
                            id: r.id,
                            bottom: parity,
                        });
                    }
                }
            }
            let selected =
                selected.ok_or_else(|| invalid("AVC field modification selects missing field"))?;
            if position > list.len() {
                return Err(invalid("missing AVC field-list entry"));
            }
            let mut tail = list.split_off(position);
            list.push(selected);
            tail.retain(|v| *v != selected);
            list.extend(tail);
        }
        if list.len() < count {
            return Err(invalid("insufficient AVC reference fields"));
        }
        list.truncate(count);
        Ok(list)
    };
    Ok(FieldLists {
        l0: modify(l0, active[0], commands[0])?,
        l1: if kind == SliceType::B {
            modify(l1, active[1], commands[1])?
        } else {
            Vec::new()
        },
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn r(id: u64, n: u32, top: Option<i32>, bottom: Option<i32>) -> FieldFrameReference {
        FieldFrameReference {
            id,
            frame_num: n,
            top_poc: top,
            bottom_poc: bottom,
            long_term_indices: [None; 2],
        }
    }
    fn s(id: u64, bottom: bool) -> FieldSelection {
        FieldSelection { id, bottom }
    }
    #[test]
    fn p_fields_alternate_parity_across_unpaired_stores_and_wrap() {
        let refs = [
            r(1, 15, Some(0), Some(1)),
            r(2, 0, Some(4), None),
            r(3, 14, None, Some(-1)),
        ];
        let top = field_lists(&refs, 4, 1, 6, false, SliceType::P, [4, 0], [&[], &[]]).unwrap();
        assert_eq!(
            top.l0,
            vec![s(2, false), s(1, true), s(1, false), s(3, true)]
        );
        let bottom = field_lists(&refs, 4, 1, 6, true, SliceType::P, [4, 0], [&[], &[]]).unwrap();
        assert_eq!(
            bottom.l0,
            vec![s(1, true), s(2, false), s(3, true), s(1, false)]
        );
    }
    #[test]
    fn b_uses_frame_poc_then_parity_and_equal_lists_swap() {
        let refs = [
            r(1, 0, Some(0), Some(10)),
            r(2, 1, Some(2), Some(3)),
            r(3, 2, Some(8), Some(9)),
        ];
        let lists = field_lists(&refs, 4, 3, 4, false, SliceType::B, [6, 6], [&[], &[]]).unwrap();
        assert_eq!(
            lists.l0,
            vec![
                s(2, false),
                s(2, true),
                s(1, false),
                s(1, true),
                s(3, false),
                s(3, true)
            ]
        );
        assert_eq!(
            lists.l1,
            vec![
                s(3, false),
                s(3, true),
                s(2, false),
                s(2, true),
                s(1, false),
                s(1, true)
            ]
        );
        let equal =
            field_lists(&refs[..1], 4, 3, 0, true, SliceType::B, [2, 2], [&[], &[]]).unwrap();
        assert_eq!(equal.l0, vec![s(1, true), s(1, false)]);
        assert_eq!(equal.l1, vec![s(1, false), s(1, true)]);
    }
    #[test]
    fn modifications_preserve_prefix_duplicates_and_select_parity() {
        let mut long = r(9, 7, Some(10), Some(11));
        long.long_term_indices = [Some(2); 2];
        let refs = [
            r(1, 0, Some(0), Some(1)),
            r(2, 15, Some(-2), Some(-1)),
            long,
        ];
        // CurrPicNum=3: subtract 2 selects same-parity frame zero;
        // a full modulo cycle deliberately selects it again.
        let mods = [
            RefModification::Subtract(1),
            RefModification::Add(31),
            RefModification::LongTerm(4),
        ];
        let lists = field_lists(&refs, 4, 1, 4, false, SliceType::P, [4, 0], [&mods, &[]]).unwrap();
        assert_eq!(
            lists.l0,
            vec![s(1, false), s(1, false), s(9, true), s(1, true)]
        );
        let bottom = field_lists(
            &refs,
            4,
            1,
            4,
            true,
            SliceType::P,
            [1, 0],
            [&[RefModification::LongTerm(5)], &[]],
        )
        .unwrap();
        assert_eq!(bottom.l0, vec![s(9, true)]);
        let wrapped = field_lists(
            &refs,
            4,
            0,
            2,
            false,
            SliceType::P,
            [1, 0],
            [&[RefModification::Subtract(1)], &[]],
        )
        .unwrap();
        assert_eq!(wrapped.l0, vec![s(2, false)]);
    }
    #[test]
    fn malformed_and_missing_field_selection_refused() {
        let refs = [r(1, 0, Some(0), None)];
        assert!(field_lists(&refs, 4, 1, 2, false, SliceType::P, [2, 0], [&[], &[]]).is_err());
        assert!(
            field_lists(
                &refs,
                4,
                1,
                2,
                false,
                SliceType::P,
                [1, 0],
                [&[RefModification::Subtract(2)], &[]]
            )
            .is_err()
        );
        assert!(
            field_lists(
                &refs,
                4,
                1,
                2,
                false,
                SliceType::P,
                [1, 0],
                [&[RefModification::Add(32)], &[]]
            )
            .is_err()
        );
        assert!(
            field_lists(
                &[r(1, 0, None, None)],
                4,
                1,
                2,
                false,
                SliceType::P,
                [1, 0],
                [&[], &[]]
            )
            .is_err()
        );
        assert!(
            field_lists(
                &[refs[0], refs[0]],
                4,
                1,
                2,
                false,
                SliceType::P,
                [1, 0],
                [&[], &[]]
            )
            .is_err()
        );
        assert!(field_lists(&refs, 3, 1, 2, false, SliceType::P, [1, 0], [&[], &[]]).is_err());
    }
    #[test]
    fn mixed_marking_and_distinct_long_indices_alternate_only_eligible_fields() {
        let mut a = r(1, 2, Some(8), Some(-10));
        a.long_term_indices = [None, Some(3)];
        let mut b = r(2, 1, Some(100), Some(4));
        b.long_term_indices = [Some(1), None];
        let mut c = r(3, 0, Some(0), Some(1));
        c.long_term_indices = [Some(0), Some(2)];
        let refs = [a, b, c];
        let top = field_lists(&refs, 4, 3, 12, false, SliceType::P, [6, 0], [&[], &[]]).unwrap();
        assert_eq!(
            top.l0,
            vec![
                s(1, false),
                s(2, true),
                s(3, false),
                s(3, true),
                s(2, false),
                s(1, true)
            ]
        );
        let bottom = field_lists(&refs, 4, 3, 12, true, SliceType::P, [6, 0], [&[], &[]]).unwrap();
        assert_eq!(
            bottom.l0,
            vec![
                s(2, true),
                s(1, false),
                s(3, true),
                s(3, false),
                s(1, true),
                s(2, false)
            ]
        );
        let changed = field_lists(
            &refs,
            4,
            3,
            12,
            false,
            SliceType::P,
            [2, 0],
            [
                &[RefModification::Subtract(1), RefModification::LongTerm(6)],
                &[],
            ],
        )
        .unwrap();
        assert_eq!(changed.l0, vec![s(1, false), s(1, true)]);
        let mut duplicate = c;
        duplicate.long_term_indices[0] = Some(1);
        assert!(
            field_lists(
                &[a, b, duplicate],
                4,
                3,
                12,
                false,
                SliceType::P,
                [1, 0],
                [&[], &[]]
            )
            .is_err()
        );
    }
    #[test]
    fn mixed_store_b_order_retains_both_field_pocs_and_opposite_long_index_reuse() {
        let mut a = r(1, 2, Some(8), Some(-10));
        a.long_term_indices = [None, Some(3)];
        let b = r(2, 1, Some(1), Some(2));
        let lists = field_lists(&[a, b], 4, 3, 4, false, SliceType::B, [4, 4], [&[], &[]]).unwrap();
        assert_eq!(
            lists.l0,
            vec![s(2, false), s(2, true), s(1, false), s(1, true)]
        );
        assert_eq!(
            lists.l1,
            vec![s(2, true), s(2, false), s(1, false), s(1, true)]
        );
        let mut c = r(3, 0, Some(0), None);
        c.long_term_indices = [Some(3), None];
        let long = field_lists(&[a, c], 4, 3, 12, false, SliceType::P, [3, 0], [&[], &[]]).unwrap();
        assert_eq!(long.l0, vec![s(1, false), s(3, false), s(1, true)]);
        let mut absent = r(4, 5, None, Some(0));
        absent.long_term_indices = [Some(0), None];
        assert!(field_lists(&[absent], 4, 3, 12, false, SliceType::P, [1, 0], [&[], &[]]).is_err());
    }
}
