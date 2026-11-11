//! Borrow compact selected fields while retaining actual field identities/POCs.
use super::super::super::{
    avc_direct::FieldDirectPrediction,
    avc_field_picture::{ImplicitFieldWeights, PcmField},
};
use super::*;
pub(super) fn decode(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[(&PcmField, u64)]; 2]],
    implicit: Option<&ImplicitFieldWeights<'_>>,
    direct: Option<&[FieldDirectPrediction<'_>]>,
    budget: usize,
    retain_motion: bool,
) -> Result<(PcmField, Option<MotionField>)> {
    let first = *headers
        .first()
        .ok_or_else(|| invalid("missing full-resolution inter field"))?;
    if references.len() != headers.len()
        || direct.is_some_and(|d| d.len() != headers.len())
        || implicit.is_some_and(|i| i.references.len() != headers.len())
    {
        return Err(invalid("444 field context coverage mismatch"));
    }
    // Only metadata is adapted; pixels stay borrowed from the selected field.
    let per_header = std::mem::size_of::<[Vec<Option<&IntraPicture>>; 2]>()
        + std::mem::size_of::<[Vec<u64>; 2]>()
        + std::mem::size_of::<[&[Option<&IntraPicture>]; 2]>()
        + std::mem::size_of::<Option<&super::super::super::avc_direct::DirectPrediction<'_>>>();
    let copies = references.iter().try_fold(
        headers
            .len()
            .checked_mul(per_header)
            .ok_or_else(|| invalid("444 field metadata overflow"))?,
        |n, lists| {
            lists[0]
                .len()
                .checked_add(lists[1].len())
                .and_then(|m| m.checked_mul(std::mem::size_of::<Option<&IntraPicture>>() + 8))
                .and_then(|m| n.checked_add(m))
                .ok_or_else(|| invalid("444 field list storage overflow"))
        },
    )?;
    let remaining = budget
        .checked_sub(copies)
        .ok_or_else(|| invalid("444 field metadata exceeds memory budget"))?;
    let mut owned = Vec::new();
    let mut identities = Vec::new();
    owned
        .try_reserve_exact(headers.len())
        .map_err(|_| invalid("cannot allocate 444 field references"))?;
    identities
        .try_reserve_exact(headers.len())
        .map_err(|_| invalid("cannot allocate 444 field identities"))?;
    for (slice, lists) in references.iter().enumerate() {
        let mut pictures: [Vec<Option<&IntraPicture>>; 2] = std::array::from_fn(|_| Vec::new());
        let mut ids: [Vec<u64>; 2] = std::array::from_fn(|_| Vec::new());
        for list in 0..2 {
            pictures[list]
                .try_reserve_exact(lists[list].len())
                .map_err(|_| invalid("cannot allocate 444 selected-field views"))?;
            ids[list]
                .try_reserve_exact(lists[list].len())
                .map_err(|_| invalid("cannot allocate 444 selected-field identities"))?;
            for &(field, id) in lists[list] {
                pictures[list].push(Some(&field.picture));
                ids[list].push(id);
            }
        }
        if let Some(contexts) = direct {
            let context = &contexts[slice];
            if context.lists[0].len() != lists[0].len()
                || context.lists[1].len() != lists[1].len()
                || context.current_bottom != first.bottom_field
                || context.inference8 != sps.direct_8x8_inference
            {
                return Err(invalid("444 field direct identity mismatch"));
            }
        }
        if let Some(context) = implicit {
            for list in 0..2 {
                if context.references[slice][list].len() != lists[list].len() {
                    return Err(invalid("444 field weight identity mismatch"));
                }
            }
        }
        owned.push(pictures);
        identities.push(ids);
    }
    let views: Vec<_> = owned
        .iter()
        .map(|lists| [lists[0].as_slice(), lists[1].as_slice()])
        .collect();
    let frames = vec![None; headers.len()];
    let context = FieldContext {
        direct,
        implicit,
        identities: &identities,
    };
    let (picture, motion) = super::decode_impl(
        headers,
        sps,
        pps,
        &views,
        &frames,
        remaining,
        Some(&context),
    )?;
    Ok((
        PcmField {
            picture,
            bottom: first.bottom_field,
            frame_num: first.frame_num,
            pps_id: first.pps_id,
        },
        retain_motion.then_some(motion),
    ))
}
