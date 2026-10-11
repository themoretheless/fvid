//! G/H.8.1.3 inter-layer DPB selection and F.7.4.7.1 reference admission.
//! This binds completed pictures to an access unit, not a slice reconstruction.
use super::{
    hevc_nal::NalHeader,
    hevc_reference_list::{LayeredReference, PictureIdentity, ReferenceKind, ReferenceSets},
    hevc_vps::LayeredVps,
};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PictureStatus {
    /// Final derived POC after any access-unit reset, never the raw coded LSB.
    pub identity: PictureIdentity,
    /// Decoder-assigned identity; POC alone does not identify an access unit.
    pub access_unit: u64,
    pub nal: NalHeader,
    pub discardable: bool,
}
impl PictureStatus {
    fn validate(&self) -> Result<()> {
        if self.identity.layer_id > 62
            || self.identity.layer_id != self.nal.layer_id
            || self.nal.temporal_id > 6
            || !matches!(self.nal.unit_type, 0..=9 | 16..=21)
            || self.nal.is_irap() && self.nal.temporal_id != 0
            || self.discardable && matches!(self.nal.unit_type, 1 | 3 | 5 | 7 | 9)
        {
            return Err(invalid("invalid HEVC inter-layer picture status"));
        }
        Ok(())
    }
}

/// Borrowed DPB pictures partitioned by ViewId. Samples and motion are still in
/// their source representation and require the appropriate G/H prediction step.
pub struct InterLayerSet<'a, T> {
    current: PictureIdentity,
    irap_current_only: bool,
    temporal_irap_forbidden: bool,
    pictures: [Vec<PictureIdentity>; 2],
    data: [Vec<&'a T>; 2],
    storage: usize,
}
impl<'a, T> InterLayerSet<'a, T> {
    /// `selected` is RefPicLayerId in coded/inferred increasing direct-layer
    /// order. Missing or ambiguous pictures in this access unit are errors;
    /// a matching POC in another access unit is never a substitute.
    pub fn derive(
        vps: &LayeredVps,
        current: PictureStatus,
        selected: &[u8],
        decoded: &'a [(PictureStatus, T)],
        budget: usize,
    ) -> Result<Self> {
        current.validate()?;
        let e = vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("HEVC inter-layer selection needs VPS extension"))?;
        let ids = &e.graph.layer_ids;
        let n = ids.len();
        if n == 0
            || n > 63
            || ids[0] != 0
            || ids.iter().any(|&id| id > 62)
            || ids.windows(2).any(|pair| pair[0] >= pair[1])
            || e.graph.direct.len() != n
            || e.graph.scalability_ids.len() != n
            || e.layer_sets.sublayers_minus1.len() != n
            || e.layer_sets.max_tid_ref_plus1.len() != n
            || e.layer_sets
                .max_tid_ref_plus1
                .iter()
                .any(|row| row.len() != n)
        {
            return Err(invalid("invalid HEVC inter-layer VPS graph"));
        }
        let index = ids
            .binary_search(&current.identity.layer_id)
            .map_err(|_| invalid("HEVC current layer is absent from VPS"))?;
        if current.nal.temporal_id > e.layer_sets.sublayers_minus1[index] {
            return Err(invalid(
                "HEVC current picture exceeds VPS temporal sublayers",
            ));
        }
        if selected.len() > 62
            || selected.windows(2).any(|pair| pair[0] >= pair[1])
            || e.representations.max_one_active_reference && selected.len() > 1
        {
            return Err(invalid(
                "invalid HEVC active inter-layer reference order/count",
            ));
        }
        // Both group vectors reserve their worst-case size, independently of
        // ViewId. Account for all four buffers before allocating any of them.
        let storage = selected
            .len()
            .checked_mul(2)
            .and_then(|n| {
                n.checked_mul(std::mem::size_of::<PictureIdentity>() + std::mem::size_of::<&T>())
            })
            .and_then(|n| {
                n.checked_add(
                    2 * std::mem::size_of::<Vec<PictureIdentity>>()
                        + 2 * std::mem::size_of::<Vec<&T>>(),
                )
            })
            .ok_or_else(|| invalid("HEVC inter-layer selection storage overflow"))?;
        if storage > budget {
            return Err(invalid("HEVC inter-layer selection exceeds memory budget"));
        }
        let view = |i: usize| -> Result<u16> {
            e.graph
                .view_ids
                .get(usize::from(e.graph.scalability_ids[i][1]))
                .copied()
                .ok_or_else(|| invalid("HEVC layer view index is absent from VPS"))
        };
        let base_view = view(0)?;
        let current_view = view(index)?;
        let mut result = Self {
            current: current.identity,
            irap_current_only: index > 0 && e.graph.direct[index] == 0 && current.nal.is_irap(),
            temporal_irap_forbidden: current.nal.unit_type == 21
                || current.nal.is_idr()
                || matches!(current.nal.unit_type, 16..=18) && e.graph.direct[index] == 0,
            pictures: std::array::from_fn(|_| Vec::with_capacity(selected.len())),
            data: std::array::from_fn(|_| Vec::with_capacity(selected.len())),
            storage,
        };
        for &layer in selected {
            let reference = ids
                .binary_search(&layer)
                .map_err(|_| invalid("HEVC reference layer is absent from VPS"))?;
            if reference >= index || e.graph.direct[index] & (1u64 << layer) == 0 {
                return Err(invalid("HEVC source is not a direct reference layer"));
            }
            let mut matches = decoded.iter().filter(|(status, _)| {
                status.access_unit == current.access_unit && status.identity.layer_id == layer
            });
            let (status, data) = matches
                .next()
                .ok_or_else(|| invalid("HEVC inter-layer reference is missing from access unit"))?;
            if matches.next().is_some() {
                return Err(invalid(
                    "HEVC inter-layer reference is ambiguous in access unit",
                ));
            }
            status.validate()?;
            if status.identity.poc != current.identity.poc {
                return Err(invalid(
                    "HEVC inter-layer reference POC disagrees with access unit",
                ));
            }
            if status.nal.temporal_id > e.layer_sets.sublayers_minus1[reference] {
                return Err(invalid(
                    "HEVC reference picture exceeds VPS temporal sublayers",
                ));
            }
            if status.nal.temporal_id != current.nal.temporal_id {
                return Err(invalid(
                    "HEVC inter-layer temporal identity disagrees with access unit",
                ));
            }
            if status.discardable {
                return Err(invalid(
                    "HEVC discardable picture cannot be an inter-layer reference",
                ));
            }
            if matches!(current.nal.unit_type, 6 | 7) && matches!(status.nal.unit_type, 8 | 9) {
                return Err(invalid(
                    "HEVC RADL picture cannot reference inter-layer RASL",
                ));
            }
            let max_tid = e.layer_sets.max_tid_ref_plus1[reference][index];
            if max_tid > 7
                || !(max_tid > current.nal.temporal_id
                    || max_tid == 0 && current.nal.temporal_id == 0 && status.nal.is_irap())
            {
                return Err(invalid(
                    "HEVC inter-layer reference exceeds temporal admission",
                ));
            }
            let reference_view = view(reference)?;
            let set0 = current_view <= base_view && current_view <= reference_view
                || current_view >= base_view && current_view >= reference_view;
            let group = usize::from(!set0);
            result.pictures[group].push(status.identity);
            result.data[group].push(data);
        }
        Ok(result)
    }
    pub fn pictures(&self) -> [&[PictureIdentity]; 2] {
        [&self.pictures[0], &self.pictures[1]]
    }
    /// Select F.8.3.4 lists while retaining the selected DPB binding within the
    /// same allocation budget. Temporal inputs are before/after/long-term.
    pub fn select(
        &self,
        temporal: [&[PictureIdentity]; 3],
        current_picture: bool,
        active: usize,
        modification: Option<&[u8]>,
        list: usize,
        budget: usize,
    ) -> Result<Vec<LayeredReference>> {
        let remaining = budget
            .checked_sub(self.storage)
            .ok_or_else(|| invalid("HEVC inter-layer lists exceed memory budget"))?;
        if self.temporal_irap_forbidden && temporal.iter().any(|set| !set.is_empty()) {
            return Err(invalid("HEVC IRAP picture cannot use temporal references"));
        }
        if self.irap_current_only && !current_picture {
            return Err(invalid(
                "HEVC independent nonbase IRAP inter lists must use only current picture",
            ));
        }
        let result = ReferenceSets {
            current: self.current,
            before: temporal[0],
            after: temporal[1],
            long_term: temporal[2],
            inter_layer: self.pictures(),
            current_picture,
        }
        .select(active, modification, list, remaining)?;
        if self.irap_current_only
            && (!current_picture || result.iter().any(|r| r.kind != ReferenceKind::Current))
        {
            return Err(invalid(
                "HEVC independent nonbase IRAP inter lists must use only current picture",
            ));
        }
        Ok(result)
    }
    pub fn resolve(&self, reference: LayeredReference) -> Result<&'a T> {
        if reference.kind != ReferenceKind::InterLayer {
            return Err(invalid("HEVC selected reference is not inter-layer"));
        }
        for group in 0..2 {
            if let Some(index) = self.pictures[group]
                .iter()
                .position(|id| *id == reference.picture)
            {
                return Ok(self.data[group][index]);
            }
        }
        Err(invalid("HEVC selected inter-layer picture is absent"))
    }
}
