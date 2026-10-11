//! H.265 8.3.4/F.8.3.4 reference-list selection, including layered identities.
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

/// POC alone does not identify an inter-layer reference picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PictureIdentity {
    pub layer_id: u8,
    pub poc: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    ShortTerm,
    LongTerm,
    InterLayer,
    Current,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayeredReference {
    pub picture: PictureIdentity,
    pub kind: ReferenceKind,
}
impl LayeredReference {
    pub fn long_term(&self) -> bool {
        self.kind != ReferenceKind::ShortTerm
    }
}
/// Groups retain their normative RPS/active-layer ordering. Before/after are
/// strictly descending/ascending POC respectively; long-term and inter-layer
/// groups retain coded order. This is list construction, not DPB availability.
pub struct ReferenceSets<'a> {
    pub current: PictureIdentity,
    pub before: &'a [PictureIdentity],
    pub after: &'a [PictureIdentity],
    pub long_term: &'a [PictureIdentity],
    pub inter_layer: [&'a [PictureIdentity]; 2],
    pub current_picture: bool,
}
impl ReferenceSets<'_> {
    pub fn select(
        &self,
        active: usize,
        modification: Option<&[u8]>,
        list: usize,
        budget: usize,
    ) -> Result<Vec<LayeredReference>> {
        let groups = [
            self.before,
            self.after,
            self.long_term,
            self.inter_layer[0],
            self.inter_layer[1],
        ];
        let decoded = groups.iter().try_fold(0usize, |n, g| {
            n.checked_add(g.len())
                .ok_or_else(|| invalid("HEVC layered reference count overflow"))
        })?;
        let total = decoded
            .checked_add(usize::from(self.current_picture))
            .ok_or_else(|| invalid("HEVC layered reference count overflow"))?;
        // list_entry syntax is stored in the decoder as an unsigned byte.
        if total > 256
            || self.current.layer_id > 62
            || list > 1
            || active > 15
            || modification.is_some_and(|m| m.len() != active)
        {
            return Err(invalid("invalid HEVC layered reference-list parameters"));
        }
        for (group, entries) in groups.iter().enumerate() {
            for p in *entries {
                if p.layer_id > 62
                    || if group < 3 {
                        p.layer_id != self.current.layer_id || p.poc == self.current.poc
                    } else {
                        p.layer_id == self.current.layer_id || p.poc != self.current.poc
                    }
                {
                    return Err(invalid(
                        "HEVC layered reference belongs to the wrong layer or POC",
                    ));
                }
                if (group == 0 && p.poc >= self.current.poc)
                    || (group == 1 && p.poc <= self.current.poc)
                {
                    return Err(invalid(
                        "HEVC short-term reference is on the wrong side of current POC",
                    ));
                }
            }
        }
        if self.before.windows(2).any(|p| p[0].poc <= p[1].poc)
            || self.after.windows(2).any(|p| p[0].poc >= p[1].poc)
        {
            return Err(invalid("HEVC short-term reference order is invalid"));
        }
        for (index, picture) in groups.iter().flat_map(|g| g.iter()).enumerate() {
            if groups
                .iter()
                .flat_map(|g| g.iter())
                .take(index)
                .any(|p| p == picture)
            {
                return Err(invalid(
                    "HEVC layered reference sets repeat a picture identity",
                ));
            }
        }
        let bytes = decoded
            .checked_add(active)
            .and_then(|n| n.checked_mul(std::mem::size_of::<LayeredReference>()))
            .and_then(|n| {
                active
                    .checked_mul(std::mem::size_of::<Source>())
                    .and_then(|s| n.checked_add(s))
            })
            .and_then(|n| n.checked_add(3 * std::mem::size_of::<Vec<LayeredReference>>()))
            .ok_or_else(|| invalid("HEVC layered reference storage overflow"))?;
        if bytes > budget {
            return Err(invalid("HEVC layered reference lists exceed memory budget"));
        }
        // Reuse ordinary selection, including repetition, explicit modifications
        // and the special unmodified/truncated L0 current-picture insertion.
        let selected = select(decoded, self.current_picture, active, modification, list)?;
        let order = if list == 0 {
            [0, 3, 1, 2, 4]
        } else {
            [1, 4, 0, 2, 3]
        };
        let mut cycle = Vec::with_capacity(decoded);
        for group in order {
            let kind = match group {
                0 | 1 => ReferenceKind::ShortTerm,
                2 => ReferenceKind::LongTerm,
                _ => ReferenceKind::InterLayer,
            };
            cycle.extend(
                groups[group]
                    .iter()
                    .map(|&picture| LayeredReference { picture, kind }),
            );
        }
        Ok(selected
            .into_iter()
            .map(|source| match source {
                Source::Decoded(i) => cycle[i],
                Source::Current => LayeredReference {
                    picture: self.current,
                    kind: ReferenceKind::Current,
                },
            })
            .collect())
    }
}

impl PictureIdentity {
    /// Resolve an exact layer/POC key without accepting an ambiguous DPB entry.
    pub fn resolve<'a, T>(&self, pictures: &'a [(Self, T)]) -> Result<&'a T> {
        if self.layer_id > 62 {
            return Err(invalid("invalid HEVC picture layer identity"));
        }
        let mut matches = pictures.iter().filter(|(id, _)| id == self);
        let value = matches
            .next()
            .ok_or_else(|| invalid("HEVC layered reference picture is missing"))?;
        if matches.next().is_some() {
            return Err(invalid(
                "HEVC layered reference picture identity is ambiguous",
            ));
        }
        Ok(&value.1)
    }
}
