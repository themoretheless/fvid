//! Annex I extended merge order and selection, I-135..I-139.
//! Inputs are already-derived available candidates in one motion/index domain.
use super::texture::MergeKind;
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Initial(usize),
    Texture,
    InterView,
    InterViewShift,
    Disparity,
    DisparityShift,
    ViewSynthesis,
}
pub struct Specials<'a, T> {
    pub texture: Option<&'a T>,
    pub inter_view: Option<&'a T>,
    pub inter_view_shift: Option<&'a T>,
    pub disparity: Option<&'a T>,
    pub disparity_shift: Option<&'a T>,
    pub view_synthesis: bool,
    pub left_view_synthesis: bool,
    pub depth: bool,
}
impl<T> Default for Specials<'_, T> {
    fn default() -> Self {
        Self {
            texture: None,
            inter_view: None,
            inter_view_shift: None,
            disparity: None,
            disparity_shift: None,
            view_synthesis: false,
            left_view_synthesis: false,
            depth: false,
        }
    }
}
fn validate<T>(initial: &[(MergeKind, T)], maximum: usize) -> Result<()> {
    if !(1..=6).contains(&maximum) || initial.len() != maximum {
        return Err(invalid("invalid HEVC extended initial merge list"));
    }
    for kind in [
        MergeKind::A1,
        MergeKind::B1,
        MergeKind::B0,
        MergeKind::A0,
        MergeKind::B2,
    ] {
        if initial.iter().filter(|(k, _)| *k == kind).count() > 1 {
            return Err(invalid("duplicate HEVC spatial merge candidate identity"));
        }
    }
    Ok(())
}
pub fn order<T: PartialEq>(
    initial: &[(MergeKind, T)],
    special: &Specials<'_, T>,
    maximum: usize,
) -> Result<Vec<Selection>> {
    let mut result = Vec::with_capacity(maximum.min(6));
    order_into(initial, special, maximum, |selection| {
        result.push(selection);
        Ok(())
    })?;
    Ok(result)
}

pub(super) fn order_into<T: PartialEq>(
    initial: &[(MergeKind, T)],
    special: &Specials<'_, T>,
    maximum: usize,
    mut emit: impl FnMut(Selection) -> Result<()>,
) -> Result<()> {
    validate(initial, maximum)?;
    let find = |kind| initial.iter().enumerate().find(|(_, (k, _))| *k == kind);
    let mut count = 0;
    let mut push = |selection| -> Result<()> {
        if count < maximum {
            emit(selection)?;
            count += 1;
        }
        Ok(())
    };
    if special.texture.is_some() {
        push(Selection::Texture)?;
    }
    if special.inter_view.is_some()
        && (special.texture.is_none() || special.inter_view != special.texture)
    {
        push(Selection::InterView)?;
    }
    let compared = if special.depth {
        special.texture
    } else {
        special.inter_view
    };
    for kind in [MergeKind::A1, MergeKind::B1] {
        if let Some((index, (_, motion))) = find(kind) {
            if compared != Some(motion) {
                push(Selection::Initial(index))?;
            }
        }
    }
    if special.view_synthesis && (find(MergeKind::A1).is_none() || !special.left_view_synthesis) {
        push(Selection::ViewSynthesis)?;
    }
    if let Some((index, _)) = find(MergeKind::B0) {
        push(Selection::Initial(index))?;
    }
    if let Some(di) = special.disparity {
        if [MergeKind::A1, MergeKind::B1]
            .iter()
            .all(|&kind| find(kind).is_none_or(|(_, (_, motion))| motion != di))
        {
            push(Selection::Disparity)?;
        }
    }
    for kind in [MergeKind::A0, MergeKind::B2] {
        if let Some((index, _)) = find(kind) {
            push(Selection::Initial(index))?;
        }
    }
    if special.inter_view_shift.is_some()
        && (special.inter_view.is_none() || special.inter_view_shift != special.inter_view)
    {
        push(Selection::InterViewShift)?;
    }
    // Availability, not actual insertion, suppresses DIShift (I-135).
    if special.disparity_shift.is_some() && special.inter_view_shift.is_none() {
        push(Selection::DisparityShift)?;
    }
    for (index, (kind, _)) in initial.iter().enumerate() {
        if *kind == MergeKind::Other {
            push(Selection::Initial(index))?;
        }
    }
    if count != maximum {
        return Err(invalid("incomplete HEVC extended merge list"));
    }
    Ok(())
}

/// I-137/I-138 and step 10: small original PBs use the initial list, but
/// selecting a VSP-coded A1 still inherits VSP when that candidate is available.
pub fn select<T: PartialEq>(
    initial: &[(MergeKind, T)],
    special: &Specials<'_, T>,
    maximum: usize,
    index: usize,
    original_size: [u32; 2],
) -> Result<Selection> {
    validate(initial, maximum)?;
    if index >= maximum || original_size.contains(&0) || original_size.iter().any(|&n| n > 64) {
        return Err(invalid("invalid HEVC extended merge selection"));
    }
    let mut selected = if original_size[0] + original_size[1] == 12 {
        Selection::Initial(index)
    } else {
        order(initial, special, maximum)?[index]
    };
    if special.view_synthesis && special.left_view_synthesis {
        if let Selection::Initial(i) = selected {
            if initial[i].0 == MergeKind::A1 {
                selected = Selection::ViewSynthesis;
            }
        }
    }
    Ok(selected)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flags {
    pub inter_view: bool,
    pub view_synthesis: bool,
    pub subblocks: bool,
}
impl Selection {
    pub fn flags(self, depth: bool, depth_partition: bool) -> Flags {
        Flags {
            inter_view: matches!(self, Self::InterView | Self::InterViewShift),
            view_synthesis: self == Self::ViewSynthesis,
            subblocks: matches!(self, Self::Texture | Self::ViewSynthesis)
                || self == Self::InterView && !depth && !depth_partition,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use MergeKind::*;
    use Selection::*;
    #[test]
    fn normative_priority_and_motion_comparisons_use_available_not_inserted_entries() {
        let initial = [(A1, 1), (B1, 2), (B0, 1), (A0, 3), (Other, 4), (Other, 5)];
        let special = Specials {
            texture: Some(&1),
            inter_view: Some(&1),
            disparity: Some(&1),
            view_synthesis: true,
            ..Default::default()
        };
        assert_eq!(
            order(&initial, &special, 6).unwrap(),
            [
                Texture,
                Initial(1),
                ViewSynthesis,
                Initial(2),
                Initial(3),
                Initial(4)
            ]
        );
        let special = Specials {
            inter_view: Some(&2),
            disparity: Some(&3),
            inter_view_shift: Some(&2),
            disparity_shift: Some(&9),
            ..Default::default()
        };
        assert_eq!(
            order(&initial, &special, 6).unwrap(),
            [
                InterView,
                Initial(0),
                Initial(2),
                Disparity,
                Initial(3),
                Initial(4)
            ]
        );
    }
    #[test]
    fn left_vsp_inheritance_and_original_small_pb_selection() {
        let initial = [(A1, 1), (B1, 2), (Other, 3)];
        let special = Specials {
            inter_view: Some(&9),
            view_synthesis: true,
            left_view_synthesis: true,
            ..Default::default()
        };
        assert_eq!(
            order(&initial, &special, 3).unwrap(),
            [InterView, Initial(0), Initial(1)]
        );
        assert_eq!(
            select(&initial, &special, 3, 1, [8, 8]).unwrap(),
            ViewSynthesis
        );
        assert_eq!(
            select(&initial, &special, 3, 0, [4, 8]).unwrap(),
            ViewSynthesis
        );
        assert_eq!(
            select(&initial, &special, 3, 1, [4, 8]).unwrap(),
            Initial(1)
        );
        assert!(select(&initial, &special, 3, 3, [8, 8]).is_err());
        assert!(order(&[(A1, 1), (A1, 2)], &Specials::default(), 2).is_err());
    }
    #[test]
    fn selected_flags_distinguish_iv_shift_and_depth_partition() {
        assert!(InterView.flags(false, false).subblocks);
        assert!(!InterView.flags(true, false).subblocks);
        assert!(!InterView.flags(false, true).subblocks);
        assert_eq!(
            InterViewShift.flags(false, false),
            Flags {
                inter_view: true,
                view_synthesis: false,
                subblocks: false
            }
        );
        assert!(ViewSynthesis.flags(false, true).subblocks);
        assert!(Texture.flags(true, false).subblocks);
    }
}
