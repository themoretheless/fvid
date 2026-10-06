//! H.265 8.3.4 reference-list selection, including the current picture.
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Decoded(usize),
    Current,
}
/// Decoded entries already have the prescribed before/after/long-term order.
pub(crate) fn select(
    decoded: usize,
    current: bool,
    active: usize,
    modification: Option<&[u8]>,
    list: usize,
) -> Result<Vec<Source>> {
    if list > 1 || active > 15 || modification.is_some_and(|m| m.len() != active) {
        return Err(invalid("invalid HEVC reference-list selection"));
    }
    let total = decoded
        .checked_add(usize::from(current))
        .ok_or_else(|| invalid("HEVC reference-list size overflow"))?;
    if total == 0 && active > 0 {
        return Err(invalid("HEVC active reference list is empty"));
    }
    let mut result = Vec::with_capacity(active);
    for i in 0..active {
        let index = modification.map_or(i % total, |m| m[i] as usize);
        if index >= total {
            return Err(invalid("HEVC reference-list index out of range"));
        }
        result.push(if index == decoded {
            Source::Current
        } else {
            Source::Decoded(index)
        });
    }
    // Unlike list 1, an unmodified truncated list 0 must still include currPic.
    if current && list == 0 && modification.is_none() && total > active && active > 0 {
        result[active - 1] = Source::Current;
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::{
        Source::{Current, Decoded},
        select,
    };
    #[test]
    fn current_only_lists_repeat_without_a_completed_picture() {
        assert_eq!(select(0, true, 3, None, 0).unwrap(), [Current; 3]);
        assert_eq!(select(0, true, 2, None, 1).unwrap(), [Current; 2]);
    }
    #[test]
    fn truncated_l0_keeps_current_but_l1_keeps_normal_prefix() {
        assert_eq!(select(3, true, 2, None, 0).unwrap(), [Decoded(0), Current]);
        assert_eq!(
            select(3, true, 2, None, 1).unwrap(),
            [Decoded(0), Decoded(1)]
        );
        assert_eq!(
            select(3, true, 5, None, 0).unwrap(),
            [Decoded(0), Decoded(1), Decoded(2), Current, Decoded(0)]
        );
    }
    #[test]
    fn explicit_modification_controls_current_selection_and_duplicates() {
        assert_eq!(
            select(3, true, 3, Some(&[3, 1, 3]), 0).unwrap(),
            [Current, Decoded(1), Current]
        );
        assert_eq!(
            select(3, true, 2, Some(&[0, 1]), 0).unwrap(),
            [Decoded(0), Decoded(1)]
        );
        assert!(select(3, true, 1, Some(&[4]), 0).is_err());
        assert!(select(3, true, 2, Some(&[0]), 0).is_err());
        assert!(select(0, false, 1, None, 0).is_err());
        assert_eq!(select(0, false, 0, None, 0).unwrap(), []);
    }
}
