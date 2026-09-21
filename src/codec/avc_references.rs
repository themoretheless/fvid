//! Frame reference-list construction (H.264 8.2.4). Field lists are not supported.
//! Entries are supplied by the decoded-picture buffer before marking the current frame.
use super::avc_slice::{RefModification, SliceType};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameReference {
    /// Stable handle into the caller's picture storage.
    pub id: u64,
    pub frame_num: u32,
    pub poc: i32,
    pub long_term_index: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReferenceLists {
    pub l0: Vec<u64>,
    pub l1: Vec<u64>,
}

/// Build and modify lists for progressive frames. `active` gives the two active
/// list lengths. Modification operands retain their slice syntax (minus one).
/// The caller must exclude non-existing pictures for this API.
pub fn frame_lists(
    references: &[FrameReference],
    frame_num_bits: u8,
    frame_num: u32,
    poc: i32,
    slice_type: SliceType,
    active: [usize; 2],
    modifications: [&[RefModification]; 2],
) -> Result<ReferenceLists> {
    if !(4..=16).contains(&frame_num_bits) || references.len() > 16 {
        return Err(invalid("invalid AVC frame reference configuration"));
    }
    let max = 1u32 << frame_num_bits;
    if frame_num >= max || active.iter().any(|&n| n > 32) {
        return Err(invalid("invalid AVC reference-list size or frame number"));
    }
    for (i, r) in references.iter().enumerate() {
        if r.frame_num >= max
            || r.long_term_index.is_some_and(|n| n >= 16)
            || references[..i].iter().any(|p| {
                p.id == r.id
                    || match (p.long_term_index, r.long_term_index) {
                        (Some(a), Some(b)) => a == b,
                        (None, None) => p.frame_num == r.frame_num,
                        _ => false,
                    }
            })
        {
            return Err(invalid("invalid or duplicate AVC reference frame"));
        }
    }
    if matches!(slice_type, SliceType::I | SliceType::Si) {
        return Ok(ReferenceLists {
            l0: Vec::new(),
            l1: Vec::new(),
        });
    }
    let pic_num = |r: &FrameReference| {
        i64::from(r.frame_num)
            - if r.frame_num > frame_num {
                i64::from(max)
            } else {
                0
            }
    };
    let mut short: Vec<_> = references
        .iter()
        .filter(|r| r.long_term_index.is_none())
        .copied()
        .collect();
    let mut long: Vec<_> = references
        .iter()
        .filter(|r| r.long_term_index.is_some())
        .copied()
        .collect();
    long.sort_by_key(|r| r.long_term_index);
    let mut l1 = Vec::new();
    if slice_type == SliceType::B {
        short.sort_by_key(|r| {
            if r.poc < poc {
                (0, -i64::from(r.poc))
            } else {
                (1, i64::from(r.poc))
            }
        });
        l1 = short.clone();
        l1.sort_by_key(|r| {
            if r.poc > poc {
                (0, i64::from(r.poc))
            } else {
                (1, -i64::from(r.poc))
            }
        });
        l1.extend_from_slice(&long);
    } else {
        short.sort_by_key(|r| -pic_num(r));
    }
    short.extend_from_slice(&long);
    if l1.len() > 1 && l1 == short {
        l1.swap(0, 1);
    }
    let modify = |mut list: Vec<FrameReference>,
                  count: usize,
                  commands: &[RefModification]|
     -> Result<Vec<u64>> {
        if count == 0 || commands.len() > count {
            return Err(invalid("invalid AVC active reference count"));
        }
        let mut predicted = i64::from(frame_num);
        for (position, command) in commands.iter().enumerate() {
            let selected = match *command {
                RefModification::LongTerm(index) => {
                    references.iter().find(|r| r.long_term_index == Some(index))
                }
                RefModification::Subtract(n) | RefModification::Add(n) => {
                    if n >= max {
                        return Err(invalid(
                            "AVC reference modification difference out of range",
                        ));
                    }
                    let delta = i64::from(n) + 1;
                    predicted = (predicted
                        + if matches!(command, RefModification::Subtract(_)) {
                            -delta
                        } else {
                            delta
                        })
                    .rem_euclid(i64::from(max));
                    let target = predicted
                        - if predicted > i64::from(frame_num) {
                            i64::from(max)
                        } else {
                            0
                        };
                    references
                        .iter()
                        .find(|r| r.long_term_index.is_none() && pic_num(r) == target)
                }
            }
            .ok_or_else(|| invalid("AVC reference modification selects missing picture"))?;
            // Duplicates in the already modified prefix are intentional and retained.
            if position > list.len() {
                return Err(invalid("missing AVC reference-list entry"));
            }
            let mut tail = list.split_off(position);
            list.push(*selected);
            tail.retain(|r| r.id != selected.id);
            list.extend(tail);
        }
        if list.len() < count {
            return Err(invalid("insufficient AVC reference pictures"));
        }
        Ok(list.into_iter().take(count).map(|r| r.id).collect())
    };
    Ok(ReferenceLists {
        l0: modify(short, active[0], modifications[0])?,
        l1: if slice_type == SliceType::B {
            modify(l1, active[1], modifications[1])?
        } else {
            Vec::new()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(id: u64, frame_num: u32, poc: i32, long: Option<u32>) -> FrameReference {
        FrameReference {
            id,
            frame_num,
            poc,
            long_term_index: long,
        }
    }
    #[test]
    fn initial_lists_wrap_and_b_order() {
        let refs = [
            r(1, 15, 0, None),
            r(2, 0, 6, None),
            r(3, 12, -4, Some(1)),
            r(4, 13, -2, Some(0)),
        ];
        let p = frame_lists(&refs, 4, 1, 8, SliceType::P, [4, 0], [&[], &[]]).unwrap();
        assert_eq!(p.l0, [2, 1, 4, 3]);
        let b = frame_lists(&refs, 4, 1, 2, SliceType::B, [4, 4], [&[], &[]]).unwrap();
        assert_eq!(b.l0, [1, 2, 4, 3]);
        assert_eq!(b.l1, [2, 1, 4, 3]);
        let b = frame_lists(&refs, 4, 1, 8, SliceType::B, [4, 4], [&[], &[]]).unwrap();
        assert_eq!(b.l0, [2, 1, 4, 3]);
        assert_eq!(b.l1, [1, 2, 4, 3]);
    }
    #[test]
    fn modifications_wrap_long_term_and_repeat() {
        let refs = [r(1, 15, 0, None), r(2, 0, 2, None), r(3, 9, -2, Some(0))];
        let commands = [
            RefModification::Subtract(1),
            RefModification::Add(0),
            RefModification::LongTerm(0),
            RefModification::Subtract(0),
        ];
        let p = frame_lists(&refs, 4, 1, 4, SliceType::P, [4, 0], [&commands, &[]]).unwrap();
        assert_eq!(p.l0, [1, 2, 3, 1]);
        assert!(
            frame_lists(
                &refs,
                4,
                1,
                4,
                SliceType::P,
                [1, 0],
                [&[RefModification::Subtract(u32::MAX)], &[]]
            )
            .is_err()
        );
        assert!(
            frame_lists(
                &refs,
                4,
                1,
                4,
                SliceType::P,
                [1, 0],
                [&[RefModification::LongTerm(2)], &[]]
            )
            .is_err()
        );
        assert!(frame_lists(&refs, 4, 1, 4, SliceType::P, [4, 0], [&[], &[]]).is_err());
    }
}
