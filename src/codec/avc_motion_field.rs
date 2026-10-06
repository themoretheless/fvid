//! Address-owned 4x4 motion storage with slice and decoding-order availability.
//! Progressive and MBAFF snapshots with explicit pair-aware publication APIs.
use super::avc_mv::{Neighbour, Neighbours, Partition, add_difference, predict};
use crate::{Result, invalid};
#[derive(Clone, Copy)]
struct Cell {
    slice: u32,
    lists: [Neighbour; 2],
    field: bool,
}
pub struct MotionField {
    width: usize,
    height: usize,
    cells: Vec<Option<Cell>>,
    mbaff: bool,
    field_picture: bool,
}
// Address-owned 4x4 cells. Spatial APIs and snapshots keep picture coordinates.
fn cell_index(width_cells: usize, x: usize, y: usize) -> usize {
    (y / 4 * (width_cells / 4) + x / 4) * 16 + y % 4 * 4 + x % 4
}
impl MotionField {
    /// Dimensions are coded luma samples. Budget covers the complete field.
    pub fn new(width: usize, height: usize, memory_limit: usize) -> Result<Self> {
        if width == 0 || height == 0 || width % 16 != 0 || height % 16 != 0 {
            return Err(invalid("invalid AVC motion-field geometry"));
        }
        let count = (width / 4)
            .checked_mul(height / 4)
            .ok_or_else(|| invalid("AVC motion-field size overflow"))?;
        if count
            .checked_mul(std::mem::size_of::<Option<Cell>>())
            .is_none_or(|n| n > memory_limit)
        {
            return Err(invalid("AVC motion field exceeds memory budget"));
        }
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate AVC motion field"))?;
        cells.resize(count, None);
        Ok(Self {
            width: width / 4,
            height: height / 4,
            cells,
            mbaff: false,
            field_picture: false,
        })
    }
    /// Compact coordinates and field motion units for a separate field picture.
    pub fn new_field(width: usize, height: usize, memory_limit: usize) -> Result<Self> {
        let mut motion = Self::new(width, height, memory_limit)?;
        motion.field_picture = true;
        Ok(motion)
    }
    /// Freeze separate-field motion with per-slice frame identities and reference
    /// parity. Do not pass parity-tagged identities: parity is stored explicitly.
    pub fn snapshot_field_slices(
        &self,
        mappings: &[(u32, [&[(u64, bool)]; 2])],
        memory_limit: usize,
    ) -> Result<super::avc_reference_motion::ReferenceMotionField> {
        use super::avc_reference_motion::{ReferenceMotion, ReferenceMotionField};
        if !self.field_picture || self.mbaff || mappings.len() > self.cells.len() / 16 {
            return Err(invalid(
                "field motion snapshot requires separate-field geometry",
            ));
        }
        let mut lists = std::collections::BTreeMap::new();
        for &(id, references) in mappings {
            if references.iter().any(|list| list.len() > 32)
                || lists.insert(id, references).is_some()
            {
                return Err(invalid(
                    "invalid or duplicate field slice reference mapping",
                ));
            }
        }
        if ReferenceMotionField::storage_bytes(self.width * 4, self.height * 4)? > memory_limit {
            return Err(invalid("field motion snapshot exceeds budget"));
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(self.cells.len())
            .map_err(|_| invalid("field motion snapshot allocation failed"))?;
        for raster in 0..self.cells.len() {
            let cell = self.cells[cell_index(self.width, raster % self.width, raster / self.width)]
                .ok_or_else(|| invalid("cannot snapshot incomplete field motion"))?;
            if !cell.field {
                return Err(invalid("field snapshot contains frame motion"));
            }
            let references = lists
                .get(&cell.slice)
                .ok_or_else(|| invalid("missing field slice reference mapping"))?;
            let mut stored = [None; 2];
            for list in 0..2 {
                stored[list] = match cell.lists[list] {
                    Neighbour::NoPrediction => None,
                    Neighbour::Inter { reference, vector } => {
                        let &(id, bottom) =
                            references[list].get(reference as usize).ok_or_else(|| {
                                invalid("field snapshot reference index out of range")
                            })?;
                        Some(ReferenceMotion {
                            picture_id: id,
                            reference_index: reference,
                            reference_bottom_field: Some(bottom),
                            vector,
                        })
                    }
                    Neighbour::Unavailable => {
                        return Err(invalid("cannot snapshot unavailable field motion"));
                    }
                };
            }
            output.push(stored);
        }
        ReferenceMotionField::new(self.width * 4, self.height * 4, output)
    }
    /// Freeze a complete picture's vectors for future co-located prediction.
    /// Slice-local indices are resolved through the supplied list identities.
    /// This entry point supports one reference-list mapping for the picture;
    /// callers must not use it for slices with different list modifications.
    pub fn snapshot(
        &self,
        references: [&[u64]; 2],
        memory_limit: usize,
    ) -> Result<super::avc_reference_motion::ReferenceMotionField> {
        if references.iter().any(|list| list.len() > 32) {
            return Err(invalid("invalid snapshot reference list"));
        }
        let mut slice = None;
        self.snapshot_resolved(memory_limit, |id| {
            if slice.is_some_and(|previous| previous != id) {
                return Err(invalid("motion snapshot needs one slice reference mapping"));
            }
            slice = Some(id);
            Ok(references)
        })
    }
    /// Resolve each slice's local list indices into persistent picture identities.
    /// The mapping must cover every decoded slice exactly once.
    pub fn snapshot_slices(
        &self,
        mappings: &[(u32, [&[u64]; 2])],
        memory_limit: usize,
    ) -> Result<super::avc_reference_motion::ReferenceMotionField> {
        if mappings.len() > self.cells.len() / 16 {
            return Err(invalid("too many slice reference mappings"));
        }
        let mut lists = std::collections::BTreeMap::new();
        for &(id, references) in mappings {
            if references.iter().any(|list| list.len() > 32)
                || lists.insert(id, references).is_some()
            {
                return Err(invalid("invalid or duplicate slice reference mapping"));
            }
        }
        self.snapshot_resolved(memory_limit, |id| {
            lists
                .get(&id)
                .copied()
                .ok_or_else(|| invalid("missing slice reference mapping"))
        })
    }
    /// Freeze MBAFF address-local cells, retaining source mode and reference parity.
    /// Frame-list identities are supplied separately for each decoded slice.
    pub fn snapshot_mbaff_slices(
        &self,
        mappings: &[(u32, [&[u64]; 2])],
        memory_limit: usize,
    ) -> Result<super::avc_reference_motion::ReferenceMotionField> {
        use super::avc_reference_motion::{ReferenceMotion, ReferenceMotionField};
        if !self.mbaff || self.height % 8 != 0 || mappings.len() > self.cells.len() / 16 {
            return Err(invalid("invalid MBAFF motion snapshot geometry"));
        }
        let mut lists = std::collections::BTreeMap::new();
        for &(id, references) in mappings {
            if references.iter().any(|l| l.len() > 32) || lists.insert(id, references).is_some() {
                return Err(invalid("invalid or duplicate MBAFF reference mapping"));
            }
        }
        let bytes = ReferenceMotionField::storage_bytes(self.width * 4, self.height * 4)?;
        if bytes > memory_limit {
            return Err(invalid("MBAFF motion snapshot exceeds budget"));
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(self.cells.len())
            .map_err(|_| invalid("MBAFF motion allocation failed"))?;
        let mut fields = Vec::new();
        fields
            .try_reserve_exact(self.cells.len() / 32)
            .map_err(|_| invalid("MBAFF pair mode allocation failed"))?;
        for (index, cell) in self.cells.iter().enumerate() {
            let cell = cell.ok_or_else(|| invalid("cannot snapshot incomplete MBAFF motion"))?;
            if index % 32 == 0 {
                fields.push(cell.field);
            }
            if fields[index / 32] != cell.field {
                return Err(invalid("MBAFF snapshot pair mode mismatch"));
            }
            let references = lists
                .get(&cell.slice)
                .ok_or_else(|| invalid("missing MBAFF slice mapping"))?;
            let mut stored = [None; 2];
            for list in 0..2 {
                stored[list] = match cell.lists[list] {
                    Neighbour::NoPrediction => None,
                    Neighbour::Unavailable => {
                        return Err(invalid("cannot snapshot unavailable MBAFF motion"));
                    }
                    Neighbour::Inter { reference, vector } => Some(ReferenceMotion {
                        picture_id: *references[list]
                            .get(usize::from(reference) / if cell.field { 2 } else { 1 })
                            .ok_or_else(|| invalid("MBAFF snapshot reference out of range"))?,
                        reference_index: reference,
                        vector,
                        reference_bottom_field: cell
                            .field
                            .then_some((index / 16 % 2) ^ (usize::from(reference) % 2) != 0),
                    }),
                };
            }
            output.push(stored);
        }
        ReferenceMotionField::new_mbaff(self.width * 4, self.height * 4, output, fields)
    }
    fn snapshot_resolved<'a>(
        &self,
        memory_limit: usize,
        mut resolve: impl FnMut(u32) -> Result<[&'a [u64]; 2]>,
    ) -> Result<super::avc_reference_motion::ReferenceMotionField> {
        if self.mbaff || self.field_picture {
            return Err(crate::unsupported(
                "frame snapshot requires frame motion geometry",
            ));
        }
        use super::avc_reference_motion::{ReferenceMotion, ReferenceMotionField};
        let bytes = ReferenceMotionField::storage_bytes(self.width * 4, self.height * 4)?;
        if bytes > memory_limit {
            return Err(invalid("reference motion snapshot exceeds budget"));
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(self.cells.len())
            .map_err(|_| invalid("reference motion allocation failed"))?;
        for raster in 0..self.cells.len() {
            let cell = self.cells[cell_index(self.width, raster % self.width, raster / self.width)]
                .ok_or_else(|| invalid("cannot snapshot incomplete motion field"))?;
            let references = resolve(cell.slice)?;
            if references.iter().any(|list| list.len() > 32) {
                return Err(invalid("invalid snapshot reference list"));
            }
            let mut stored = [None; 2];
            for list in 0..2 {
                stored[list] = match cell.lists[list] {
                    Neighbour::NoPrediction => None,
                    Neighbour::Inter { reference, vector } => Some(ReferenceMotion {
                        picture_id: *references[list]
                            .get(reference as usize)
                            .ok_or_else(|| invalid("snapshot reference index out of range"))?,
                        reference_index: reference,
                        reference_bottom_field: None,
                        vector,
                    }),
                    Neighbour::Unavailable => {
                        return Err(invalid("cannot snapshot unavailable motion"));
                    }
                };
            }
            output.push(stored);
        }
        ReferenceMotionField::new(self.width * 4, self.height * 4, output)
    }
    fn region(&self, origin: [usize; 2], size: [usize; 2]) -> Result<([usize; 2], [usize; 2])> {
        if ![4, 8, 16].contains(&size[0])
            || ![4, 8, 16].contains(&size[1])
            || origin.iter().any(|n| n % 4 != 0)
        {
            return Err(invalid("invalid AVC motion partition"));
        }
        let at = [origin[0] / 4, origin[1] / 4];
        let extent = [size[0] / 4, size[1] / 4];
        if at[0] > self.width - extent[0]
            || at[1] > self.height - extent[1]
            || at[0] % 4 + extent[0] > 4
            || at[1] % 4 + extent[1] > 4
        {
            return Err(invalid("AVC motion partition crosses macroblock boundary"));
        }
        Ok((at, extent))
    }
    pub fn neighbours(
        &self,
        origin: [usize; 2],
        size: [usize; 2],
        slice: u32,
        list: usize,
    ) -> Result<Neighbours> {
        if self.mbaff {
            return Err(invalid("MBAFF motion needs pair-address neighbour lookup"));
        }
        if list > 1 {
            return Err(invalid("invalid AVC reference list"));
        }
        let ([x, y], _) = self.region(origin, size)?;
        let neighbours = super::avc_mbaff::motion_neighbours(
            y / 4 * (self.width / 4) + x / 4,
            [x % 4 * 4, y % 4 * 4],
            size,
            self.width / 4,
            self.height / 4,
            false,
            |_| None,
        )?;
        let [left, top, top_right, top_left] = neighbours.map(|n| {
            n.and_then(|(address, local)| self.cells[address * 16 + local[1] * 4 + local[0]])
                .filter(|cell| cell.slice == slice)
                .map_or(Neighbour::Unavailable, |cell| cell.lists[list])
        });
        Ok(Neighbours {
            left,
            top,
            top_right,
            top_left,
        })
    }
    /// Publish a completely decoded partition. Available intra/unused-list cells
    /// use NoPrediction, never Unavailable. Duplicate writes are rejected.
    pub fn store(
        &mut self,
        origin: [usize; 2],
        size: [usize; 2],
        slice: u32,
        lists: [Neighbour; 2],
    ) -> Result<()> {
        if self.mbaff {
            return Err(invalid("MBAFF motion needs pair-address publication"));
        }
        if self.field_picture
            && lists
                .iter()
                .any(|n| matches!(n,Neighbour::Inter{reference,..} if *reference>31))
        {
            return Err(invalid(
                "separate field motion reference exceeds active-list range",
            ));
        }
        self.store_with_mode(origin, size, slice, lists, self.field_picture)
    }
    fn store_with_mode(
        &mut self,
        origin: [usize; 2],
        size: [usize; 2],
        slice: u32,
        lists: [Neighbour; 2],
        field: bool,
    ) -> Result<()> {
        let ([x, y], [w, h]) = self.region(origin, size)?;
        if lists.iter().any(|n| {
            matches!(n, Neighbour::Unavailable)
                || matches!(n,Neighbour::Inter{reference,..} if *reference>if field {63} else {31})
        }) {
            return Err(invalid("invalid decoded AVC motion value"));
        }
        for row in y..y + h {
            for col in x..x + w {
                if self.cells[cell_index(self.width, col, row)].is_some() {
                    return Err(invalid("AVC motion partition already decoded"));
                }
            }
        }
        for row in y..y + h {
            for col in x..x + w {
                self.cells[cell_index(self.width, col, row)] = Some(Cell {
                    slice,
                    lists,
                    field,
                });
            }
        }
        Ok(())
    }
    /// Publish local luma coordinates in a macroblock-pair address space.
    /// Pair-mode ownership is maintained by the slice reader. Snapshot support
    /// is separate: progressive snapshots reject a field containing MBAFF data.
    pub fn store_mbaff(
        &mut self,
        address: usize,
        origin: [usize; 2],
        size: [usize; 2],
        slice: u32,
        field: bool,
        lists: [Neighbour; 2],
    ) -> Result<()> {
        let w = self.width / 4;
        super::avc_mbaff::layout(address, w, self.height / 4, true, field, [1, 1])?;
        if !self.mbaff && self.cells.iter().any(Option::is_some) {
            return Err(invalid(
                "cannot mix progressive and MBAFF motion publication",
            ));
        }
        if origin.iter().any(|v| *v >= 16) {
            return Err(invalid("MBAFF motion origin outside block"));
        }
        let absolute = [address % w * 16 + origin[0], address / w * 16 + origin[1]];
        let pair_start = address / 2 * 32;
        if self.cells[pair_start..pair_start + 32]
            .iter()
            .flatten()
            .any(|c| c.field != field)
        {
            return Err(invalid("MBAFF motion pair mode changed"));
        }
        self.store_with_mode(absolute, size, slice, lists, field)?;
        self.mbaff = true;
        Ok(())
    }
    /// Resolve and normalize A/B/C/D using the slice reader's pair modes.
    pub fn neighbours_mbaff(
        &self,
        address: usize,
        origin: [usize; 2],
        size: [usize; 2],
        slice: u32,
        list: usize,
        mut pair_field: impl FnMut(usize) -> Option<bool>,
    ) -> Result<Neighbours> {
        if list > 1 {
            return Err(invalid("invalid AVC reference list"));
        }
        if !self.mbaff && self.cells.iter().any(Option::is_some) {
            return Err(invalid("cannot query progressive motion as MBAFF"));
        }
        let locations = super::avc_mbaff::motion_neighbours(
            address,
            origin,
            size,
            self.width / 4,
            self.height / 4,
            true,
            &mut pair_field,
        )?;
        let current = pair_field(address / 2);
        let mut values = [Neighbour::Unavailable; 4];
        if let Some(field) = current {
            for (slot, location) in values.iter_mut().zip(locations) {
                if let Some((owner, local)) = location {
                    if let Some(cell) = self.cells[owner * 16 + local[1] * 4 + local[0]]
                        .filter(|c| c.slice == slice)
                    {
                        if pair_field(owner / 2) != Some(cell.field) {
                            return Err(invalid("MBAFF motion pair mode mismatch"));
                        }
                        *slot = super::avc_mv::normalize_neighbour(
                            cell.lists[list],
                            field,
                            cell.field,
                        )?;
                    }
                }
            }
        }
        let [left, top, top_right, top_left] = values;
        Ok(Neighbours {
            left,
            top,
            top_right,
            top_left,
        })
    }
    /// Derive both explicit list vectors in the current pair's units before
    /// publishing any cell. A failed second list leaves the partition untouched.
    pub fn decode_mbaff(
        &mut self,
        address: usize,
        origin: [usize; 2],
        size: [usize; 2],
        slice: u32,
        kind: Partition,
        references: [Option<u8>; 2],
        differences: [[i32; 2]; 2],
        mut pair_field: impl FnMut(usize) -> Option<bool>,
    ) -> Result<[Neighbour; 2]> {
        let field =
            pair_field(address / 2).ok_or_else(|| invalid("MBAFF partition lacks pair mode"))?;
        // Validate geometry and ownership even when neither list is used.
        let first = self.neighbours_mbaff(address, origin, size, slice, 0, &mut pair_field)?;
        let mut lists = [Neighbour::NoPrediction; 2];
        for list in 0..2 {
            if let Some(reference) = references[list] {
                let neighbours = if list == 0 {
                    first
                } else {
                    self.neighbours_mbaff(address, origin, size, slice, list, &mut pair_field)?
                };
                let prediction =
                    super::avc_mv::predict_for_field(reference, kind, neighbours, field)?;
                lists[list] = Neighbour::Inter {
                    reference,
                    vector: add_difference(prediction, differences[list])?,
                };
            }
        }
        self.store_mbaff(address, origin, size, slice, field, lists)?;
        Ok(lists)
    }
    /// Decode explicit MBAFF partitions in syntax order, rolling back the whole
    /// address-local macroblock and storage mode on any failure.
    pub fn decode_macroblock_mbaff(
        &mut self,
        address: usize,
        slice: u32,
        parts: &[super::avc_inter::Partition],
        pair_field: impl FnMut(usize) -> Option<bool>,
    ) -> Result<Vec<[Neighbour; 2]>> {
        self.decode_macroblock_mbaff_with_direct(address, slice, parts, pair_field, None)
    }
    /// Mixed explicit/direct MBAFF motion is one address-local transaction.
    /// Spatial direct always uses the macroblock's original external neighbours,
    /// regardless of earlier explicit/direct subpartition publication.
    pub fn decode_macroblock_mbaff_with_direct(
        &mut self,
        address: usize,
        slice: u32,
        parts: &[super::avc_inter::Partition],
        mut pair_field: impl FnMut(usize) -> Option<bool>,
        direct: Option<&super::avc_direct::MbaffDirectPrediction<'_>>,
    ) -> Result<Vec<[Neighbour; 2]>> {
        use super::avc_inter::Prediction;
        let field =
            pair_field(address / 2).ok_or_else(|| invalid("MBAFF macroblock lacks pair mode"))?;
        super::avc_mbaff::layout(
            address,
            self.width / 4,
            self.height / 4,
            true,
            field,
            [1, 1],
        )?;
        if parts.is_empty() || parts.len() > 16 {
            return Err(invalid("invalid AVC inter macroblock partitions"));
        }
        let start = address * 16;
        if self.cells[start..start + 16].iter().any(Option::is_some) {
            return Err(invalid("AVC macroblock already has motion data"));
        }
        let direct_neighbours = if parts.iter().any(|p| p.prediction == Prediction::Direct) {
            let context = direct.ok_or_else(|| invalid("missing MBAFF B-direct context"))?;
            if context
                .colocated
                .is_some_and(|c| c.dimensions() != [self.width * 4, self.height * 4])
            {
                return Err(invalid("MBAFF co-located motion geometry mismatch"));
            }
            Some([
                self.neighbours_mbaff(address, [0, 0], [16, 16], slice, 0, &mut pair_field)?,
                self.neighbours_mbaff(address, [0, 0], [16, 16], slice, 1, &mut pair_field)?,
            ])
        } else {
            None
        };
        let old_mode = self.mbaff;
        let decoded = (|| {
            let mut result = Vec::with_capacity(parts.len());
            for p in parts {
                if p.prediction == Prediction::Direct {
                    if p.size != [4, 4] || p.references != [None; 2] || p.differences != [[0; 2]; 2]
                    {
                        return Err(invalid("invalid MBAFF direct partition syntax"));
                    }
                    let vectors = direct
                        .ok_or_else(|| invalid("missing MBAFF B-direct context"))?
                        .derive(
                            address,
                            p.origin.map(usize::from),
                            field,
                            direct_neighbours
                                .ok_or_else(|| invalid("missing MBAFF direct neighbours"))?,
                        )?;
                    self.store_mbaff(
                        address,
                        p.origin.map(usize::from),
                        [4, 4],
                        slice,
                        field,
                        vectors,
                    )?;
                    result.push(vectors);
                    continue;
                }
                let expected = match p.prediction {
                    Prediction::L0 => [true, false],
                    Prediction::L1 => [false, true],
                    Prediction::Bi => [true, true],
                    Prediction::Direct => {
                        return Err(invalid("invalid MBAFF direct dispatch"));
                    }
                };
                if p.references.map(|v| v.is_some()) != expected {
                    return Err(invalid(
                        "AVC partition references disagree with prediction mode",
                    ));
                }
                let size = p.size.map(usize::from);
                let kind = match size {
                    [16, 8] if p.origin[1] == 0 => Partition::Top16x8,
                    [16, 8] => Partition::Bottom16x8,
                    [8, 16] if p.origin[0] == 0 => Partition::Left8x16,
                    [8, 16] => Partition::Right8x16,
                    _ => Partition::Median,
                };
                result.push(self.decode_mbaff(
                    address,
                    p.origin.map(usize::from),
                    size,
                    slice,
                    kind,
                    p.references,
                    p.differences,
                    &mut pair_field,
                )?);
            }
            if self.cells[start..start + 16].iter().any(Option::is_none) {
                return Err(invalid("AVC motion partitions leave gaps"));
            }
            Ok(result)
        })();
        if decoded.is_err() {
            self.cells[start..start + 16].fill(None);
            self.mbaff = old_mode;
        }
        decoded
    }
    /// Derive and publish a complete MBAFF P-skip block. The slice reader owns
    /// skipped-pair field-mode inference; an unknown current mode is an error.
    pub fn decode_p_skip_mbaff(
        &mut self,
        address: usize,
        slice: u32,
        mut pair_field: impl FnMut(usize) -> Option<bool>,
    ) -> Result<[i16; 2]> {
        let neighbours =
            self.neighbours_mbaff(address, [0, 0], [16, 16], slice, 0, &mut pair_field)?;
        let field = pair_field(address / 2).ok_or_else(|| invalid("MBAFF skip lacks pair mode"))?;
        let vector = super::avc_mv::p_skip_for_field(neighbours, field)?;
        self.store_mbaff(
            address,
            [0, 0],
            [16, 16],
            slice,
            field,
            [
                Neighbour::Inter {
                    reference: 0,
                    vector,
                },
                Neighbour::NoPrediction,
            ],
        )?;
        Ok(vector)
    }
    /// Derive both non-direct list vectors, then publish the partition atomically.
    /// P-skip/B-direct use their own derivation and call store with final vectors.
    pub fn decode(
        &mut self,
        origin: [usize; 2],
        size: [usize; 2],
        slice: u32,
        kind: Partition,
        references: [Option<u8>; 2],
        differences: [[i32; 2]; 2],
    ) -> Result<[Neighbour; 2]> {
        let mut lists = [Neighbour::NoPrediction; 2];
        for list in 0..2 {
            if let Some(reference) = references[list] {
                let prediction =
                    predict(reference, kind, self.neighbours(origin, size, slice, list)?)?;
                lists[list] = Neighbour::Inter {
                    reference,
                    vector: add_difference(prediction, differences[list])?,
                };
            }
        }
        self.store(origin, size, slice, lists)?;
        Ok(lists)
    }
    /// Decode an entire non-direct macroblock in bitstream partition order.
    /// Any error restores all sixteen cells, including earlier partitions.
    pub fn decode_macroblock(
        &mut self,
        origin: [usize; 2],
        slice: u32,
        parts: &[super::avc_inter::Partition],
    ) -> Result<Vec<[Neighbour; 2]>> {
        self.decode_macroblock_with_direct(origin, slice, parts, None)
    }
    /// Decode mixed explicit/direct partitions, rolling back the entire macroblock
    /// on failure. Direct predictors always use the original macroblock neighbours.
    pub fn decode_macroblock_with_direct(
        &mut self,
        origin: [usize; 2],
        slice: u32,
        parts: &[super::avc_inter::Partition],
        direct: Option<&super::avc_direct::DirectPrediction<'_>>,
    ) -> Result<Vec<[Neighbour; 2]>> {
        use super::avc_inter::Prediction;
        if origin.iter().any(|n| n % 16 != 0) || parts.is_empty() || parts.len() > 16 {
            return Err(invalid("invalid AVC inter macroblock partitions"));
        }
        let ([x, y], _) = self.region(origin, [16, 16])?;
        let previous: [Option<Cell>; 16] =
            std::array::from_fn(|i| self.cells[cell_index(self.width, x + i % 4, y + i / 4)]);
        if previous.iter().any(Option::is_some) {
            return Err(invalid("AVC macroblock already has motion data"));
        }
        let direct_neighbours = [
            self.neighbours(origin, [16, 16], slice, 0)?,
            self.neighbours(origin, [16, 16], slice, 1)?,
        ];
        let decoded = (|| {
            let mut result = Vec::with_capacity(parts.len());
            for p in parts {
                if p.origin.iter().any(|&n| n >= 16) {
                    return Err(invalid("AVC partition origin outside macroblock"));
                }
                if p.prediction == Prediction::Direct {
                    if p.size != [4, 4] || p.references != [None; 2] || p.differences != [[0; 2]; 2]
                    {
                        return Err(invalid("invalid direct partition syntax"));
                    }
                    let at = [
                        origin[0] + usize::from(p.origin[0]),
                        origin[1] + usize::from(p.origin[1]),
                    ];
                    let vectors = direct
                        .ok_or_else(|| invalid("missing B-direct context"))?
                        .derive(at, direct_neighbours)?;
                    self.store(at, [4, 4], slice, vectors)?;
                    result.push(vectors);
                    continue;
                }
                let expected = match p.prediction {
                    Prediction::L0 => [true, false],
                    Prediction::L1 => [false, true],
                    Prediction::Bi => [true, true],
                    Prediction::Direct => {
                        return Err(invalid("B-direct motion derivation is not connected"));
                    }
                };
                if [p.references[0].is_some(), p.references[1].is_some()] != expected {
                    return Err(invalid(
                        "AVC partition references disagree with prediction mode",
                    ));
                }
                let size = p.size.map(usize::from);
                let kind = match size {
                    [16, 8] => {
                        if p.origin[1] == 0 {
                            Partition::Top16x8
                        } else {
                            Partition::Bottom16x8
                        }
                    }
                    [8, 16] => {
                        if p.origin[0] == 0 {
                            Partition::Left8x16
                        } else {
                            Partition::Right8x16
                        }
                    }
                    _ => Partition::Median,
                };
                result.push(self.decode(
                    [
                        origin[0] + usize::from(p.origin[0]),
                        origin[1] + usize::from(p.origin[1]),
                    ],
                    size,
                    slice,
                    kind,
                    p.references,
                    p.differences,
                )?);
            }
            if (0..16).any(|i| self.cells[cell_index(self.width, x + i % 4, y + i / 4)].is_none()) {
                return Err(invalid("AVC motion partitions leave gaps"));
            }
            Ok(result)
        })();
        if decoded.is_err() {
            for (i, cell) in previous.into_iter().enumerate() {
                self.cells[cell_index(self.width, x + i % 4, y + i / 4)] = cell;
            }
        }
        decoded
    }
    pub fn decode_p_skip(&mut self, origin: [usize; 2], slice: u32) -> Result<[i16; 2]> {
        if origin.iter().any(|n| n % 16 != 0) {
            return Err(invalid("unaligned AVC P-skip macroblock"));
        }
        let vector = super::avc_mv::p_skip(self.neighbours(origin, [16, 16], slice, 0)?)?;
        self.store(
            origin,
            [16, 16],
            slice,
            [
                Neighbour::Inter {
                    reference: 0,
                    vector,
                },
                Neighbour::NoPrediction,
            ],
        )?;
        Ok(vector)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prediction_order_slice_isolation_and_atomic_failure() {
        let mut field = MotionField::new(32, 16, 8192).unwrap();
        let first = field
            .decode(
                [0, 0],
                [16, 16],
                1,
                Partition::Median,
                [Some(0), None],
                [[8, -4], [0, 0]],
            )
            .unwrap();
        let n = field.neighbours([16, 0], [16, 16], 1, 0).unwrap();
        assert_eq!(n.left, first[0]);
        assert_eq!(
            field.neighbours([16, 0], [16, 16], 2, 0).unwrap().left,
            Neighbour::Unavailable
        );
        assert!(
            field
                .decode(
                    [16, 0],
                    [16, 16],
                    1,
                    Partition::Median,
                    [Some(0), None],
                    [[i32::MAX, 0], [0, 0]]
                )
                .is_err()
        );
        let second = field
            .decode(
                [16, 0],
                [16, 16],
                1,
                Partition::Median,
                [Some(0), None],
                [[1, 2], [0, 0]],
            )
            .unwrap();
        assert_eq!(
            second[0],
            Neighbour::Inter {
                reference: 0,
                vector: [9, -2]
            }
        );
        assert!(field.store([16, 0], [16, 16], 1, first).is_err());
        assert!(MotionField::new(16, 16, 1).is_err());
        assert!(field.neighbours([usize::MAX, 0], [16, 16], 1, 0).is_err());
    }
}

#[cfg(test)]
mod macroblock_tests {
    use super::super::{avc_inter::read_prediction, avc_slice::SliceType, bits::BitReader};
    use super::*;
    #[test]
    fn entropy_partitions_connect_to_motion_and_rollback() {
        let mut field = MotionField::new(32, 16, 8192).unwrap();
        // P_8x8ref0, all four subtypes 8x8, all motion differences zero.
        let mut bits = BitReader::new(&[0xff, 0xf0]);
        let parts = read_prediction(&mut bits, SliceType::P, 4, [1, 0]).unwrap();
        let mut invalid = parts.clone();
        invalid[3].differences[0][0] = i32::MAX;
        assert!(field.decode_macroblock([0, 0], 0, &invalid).is_err());
        let decoded = field.decode_macroblock([0, 0], 0, &parts).unwrap();
        assert_eq!(decoded.len(), 4);
        assert!(decoded.iter().all(|p| p[0]
            == Neighbour::Inter {
                reference: 0,
                vector: [0, 0]
            }));
        assert_eq!(field.decode_p_skip([16, 0], 0).unwrap(), [0, 0]);
    }
}

#[cfg(test)]
mod slice_snapshot_tests {
    use super::*;
    #[test]
    fn mbaff_temporal_direct_snapshot_retains_derived_field_reference_identity() {
        use super::super::{
            avc_direct::MbaffDirectPrediction,
            avc_inter::{MacroblockType, macroblock_type},
            avc_poc::FieldOrder,
            avc_reference_motion::{ReferenceMotion, ReferenceMotionField},
            avc_references::FrameReference,
            avc_slice::SliceType,
        };
        let MacroblockType::Inter { partitions, .. } = macroblock_type(SliceType::B, 0).unwrap()
        else {
            panic!()
        };
        let list0 = [FrameReference {
            id: 42,
            frame_num: 0,
            poc: 0,
            long_term_index: None,
        }];
        let list1 = [FrameReference {
            id: 70,
            frame_num: 1,
            poc: 8,
            long_term_index: None,
        }];
        let orders0 = [FieldOrder {
            top: Some(0),
            bottom: Some(4),
        }];
        let orders1 = [FieldOrder {
            top: Some(8),
            bottom: Some(12),
        }];
        let source = ReferenceMotionField::new_mbaff(
            16,
            32,
            (0..32)
                .map(|cell| {
                    [
                        Some(ReferenceMotion {
                            picture_id: 42,
                            reference_index: 0,
                            reference_bottom_field: Some(false),
                            vector: [(cell % 16 * 4) as i16, 4],
                        }),
                        None,
                    ]
                })
                .collect(),
            vec![true],
        )
        .unwrap();
        let context = MbaffDirectPrediction {
            spatial: false,
            inference8: true,
            current_order: FieldOrder {
                top: Some(2),
                bottom: Some(8),
            },
            list0: &list0,
            list1: &list1,
            list0_orders: &orders0,
            list1_orders: &orders1,
            colocated: Some(&source),
        };
        let mut working = MotionField::new(16, 32, 65536).unwrap();
        for address in 0..2 {
            working
                .decode_macroblock_mbaff_with_direct(
                    address,
                    5,
                    &partitions,
                    |_| Some(true),
                    Some(&context),
                )
                .unwrap();
        }
        let saved = working
            .snapshot_mbaff_slices(&[(5, [&[42], &[70]])], 65536)
            .unwrap();
        for (bottom, vector, reference) in [(0, [15, 1], 0), (1, [40, 3], 1)] {
            let (lists, field) = saved.at_mbaff([12, 24 + bottom]).unwrap();
            assert!(field);
            assert_eq!(
                lists[0].unwrap(),
                ReferenceMotion {
                    picture_id: 42,
                    reference_index: reference,
                    reference_bottom_field: Some(false),
                    vector
                }
            );
            assert_eq!(
                lists[1].unwrap(),
                ReferenceMotion {
                    picture_id: 70,
                    reference_index: 0,
                    reference_bottom_field: Some(bottom != 0),
                    vector: [vector[0] - 60, vector[1] - 4]
                }
            );
        }
    }
    #[test]
    fn mbaff_direct_publication_is_complete_and_late_failures_restore_storage() {
        use super::super::{
            avc_direct::MbaffDirectPrediction,
            avc_inter::{MacroblockType, Prediction, macroblock_type},
            avc_poc::FieldOrder,
            avc_references::FrameReference,
            avc_slice::SliceType,
        };
        let MacroblockType::Inter { partitions, .. } = macroblock_type(SliceType::B, 0).unwrap()
        else {
            panic!()
        };
        let references = [FrameReference {
            id: 42,
            frame_num: 0,
            poc: 0,
            long_term_index: None,
        }];
        let orders = [FieldOrder {
            top: Some(0),
            bottom: Some(2),
        }];
        let context = MbaffDirectPrediction {
            spatial: true,
            inference8: true,
            current_order: FieldOrder {
                top: Some(4),
                bottom: Some(6),
            },
            list0: &references,
            list1: &references,
            list0_orders: &orders,
            list1_orders: &orders,
            colocated: None,
        };
        for field_mode in [false, true] {
            let mut field = MotionField::new(16, 32, 65536).unwrap();
            let mut mixed = partitions.clone();
            mixed[0].prediction = Prediction::L0;
            mixed[0].references = [Some(0), None];
            mixed[0].differences = [[100, -7], [0, 0]];
            let mut damaged = mixed.clone();
            damaged.last_mut().unwrap().references = [Some(0), None];
            assert!(
                field
                    .decode_macroblock_mbaff_with_direct(
                        0,
                        3,
                        &damaged,
                        |_| Some(field_mode),
                        Some(&context)
                    )
                    .is_err()
            );
            assert!(!field.mbaff);
            assert!(field.cells.iter().all(Option::is_none));
            assert!(
                field
                    .decode_macroblock_mbaff_with_direct(
                        0,
                        3,
                        &mixed[..15],
                        |_| Some(field_mode),
                        Some(&context)
                    )
                    .is_err()
            );
            assert!(!field.mbaff);
            assert!(field.cells.iter().all(Option::is_none));
            let output = field
                .decode_macroblock_mbaff_with_direct(
                    0,
                    3,
                    &mixed,
                    |_| Some(field_mode),
                    Some(&context),
                )
                .unwrap();
            assert_eq!(
                output[0],
                [
                    Neighbour::Inter {
                        reference: 0,
                        vector: [100, -7]
                    },
                    Neighbour::NoPrediction
                ]
            );
            // Earlier explicit cells must not contaminate spatial direct's
            // macroblock-partition-0 external neighbours.
            assert!(output[1..].iter().all(|v| *v
                == [Neighbour::Inter {
                    reference: 0,
                    vector: [0, 0]
                }; 2]));
            assert!(
                field.cells[..16]
                    .iter()
                    .all(|c| c.is_some_and(|c| c.field == field_mode && c.slice == 3))
            );
            let saved: Vec<_> = field.cells[..16].iter().map(|c| c.unwrap().lists).collect();
            assert!(
                field
                    .decode_macroblock_mbaff_with_direct(
                        1,
                        3,
                        &damaged,
                        |_| Some(field_mode),
                        Some(&context)
                    )
                    .is_err()
            );
            assert!(field.mbaff);
            assert!(field.cells[16..].iter().all(Option::is_none));
            assert_eq!(
                field.cells[..16]
                    .iter()
                    .map(|c| c.unwrap().lists)
                    .collect::<Vec<_>>(),
                saved
            );
            let decoded = field
                .decode_macroblock_mbaff_with_direct(
                    1,
                    4,
                    &partitions,
                    |_| Some(field_mode),
                    Some(&context),
                )
                .unwrap();
            assert!(decoded.iter().all(|v| *v
                == [Neighbour::Inter {
                    reference: 0,
                    vector: [0, 0]
                }; 2]));
            let snapshot = field
                .snapshot_mbaff_slices(&[(3, [&[42], &[42]]), (4, [&[42], &[42]])], 65536)
                .unwrap();
            assert!(snapshot.is_mbaff());
        }
        let mut field = MotionField::new(16, 32, 65536).unwrap();
        assert!(
            field
                .decode_macroblock_mbaff_with_direct(0, 0, &partitions, |_| Some(true), None)
                .is_err()
        );
        assert!(field.cells.iter().all(Option::is_none));
    }
    #[test]
    fn mbaff_macroblock_late_failure_rolls_back_cells_and_mode() {
        use super::super::avc_inter::{Partition as SyntaxPartition, Prediction};
        let top = SyntaxPartition {
            origin: [0, 0],
            size: [16, 8],
            prediction: Prediction::L0,
            group: 0,
            references: [Some(63), None],
            differences: [[7, -3], [0; 2]],
        };
        let mut bottom = SyntaxPartition {
            origin: [0, 8],
            group: 1,
            ..top
        };
        let mut field = MotionField::new(16, 32, 65536).unwrap();
        bottom.differences[0] = [i32::MAX, 0];
        assert!(
            field
                .decode_macroblock_mbaff(0, 4, &[top, bottom], |_| Some(true))
                .is_err()
        );
        assert!(!field.mbaff);
        assert!(field.cells.iter().all(Option::is_none));
        assert!(
            field
                .decode_macroblock_mbaff(0, 4, &[top], |_| Some(true))
                .is_err()
        );
        assert!(!field.mbaff);
        bottom.differences[0] = [1, 2];
        let result = field
            .decode_macroblock_mbaff(0, 4, &[top, bottom], |_| Some(true))
            .unwrap();
        assert_eq!(
            result[1][0],
            Neighbour::Inter {
                reference: 63,
                vector: [8, -1]
            }
        );
        assert!(field.cells[..16].iter().all(Option::is_some));
        let saved = field.cells[0].unwrap().lists;
        assert!(
            field
                .decode_macroblock_mbaff(0, 4, &[top, bottom], |_| Some(true))
                .is_err()
        );
        assert_eq!(field.cells[0].unwrap().lists, saved);
        bottom.prediction = Prediction::Direct;
        assert!(
            field
                .decode_macroblock_mbaff(1, 4, &[top, bottom], |_| Some(true))
                .is_err()
        );
        assert!(field.mbaff);
        assert!(field.cells[16..].iter().all(Option::is_none));
        assert_eq!(field.cells[0].unwrap().lists, saved);
    }
    #[test]
    fn mbaff_explicit_lists_normalize_then_add_and_publish_atomically() {
        let mut field = MotionField::new(32, 32, 65536).unwrap();
        for (address, y) in [(0, -5), (1, -7)] {
            field
                .store_mbaff(
                    address,
                    [0, 0],
                    [16, 16],
                    7,
                    false,
                    [
                        Neighbour::Inter {
                            reference: 1,
                            vector: [3, y],
                        },
                        Neighbour::Inter {
                            reference: 2,
                            vector: [10, y],
                        },
                    ],
                )
                .unwrap();
        }
        let mode = |p| Some(p == 1);
        assert!(
            field
                .decode_mbaff(
                    2,
                    [0, 8],
                    [8, 8],
                    7,
                    Partition::Median,
                    [Some(2), Some(4)],
                    [[1, 2], [i32::MAX, 0]],
                    mode
                )
                .is_err()
        );
        assert!(field.cells[32..64].iter().all(Option::is_none));
        let lists = field
            .decode_mbaff(
                2,
                [0, 8],
                [8, 8],
                7,
                Partition::Median,
                [Some(2), Some(4)],
                [[1, 2], [-2, 1]],
                mode,
            )
            .unwrap();
        assert_eq!(
            lists,
            [
                Neighbour::Inter {
                    reference: 2,
                    vector: [4, 0]
                },
                Neighbour::Inter {
                    reference: 4,
                    vector: [8, -1]
                },
            ]
        );
        for index in [40, 41, 44, 45] {
            assert_eq!(field.cells[index].unwrap().lists, lists);
        }
        assert!(
            field
                .decode_mbaff(
                    2,
                    [0, 8],
                    [8, 8],
                    7,
                    Partition::Median,
                    [Some(2), None],
                    [[0; 2]; 2],
                    mode
                )
                .is_err()
        );
        assert!(
            field
                .decode_mbaff(
                    3,
                    [0, 0],
                    [4, 4],
                    7,
                    Partition::Median,
                    [None, None],
                    [[0; 2]; 2],
                    |_| None
                )
                .is_err()
        );
        assert!(
            field
                .decode_mbaff(
                    3,
                    [usize::MAX, 0],
                    [4, 4],
                    7,
                    Partition::Median,
                    [None, None],
                    [[0; 2]; 2],
                    mode
                )
                .is_err()
        );
        assert!(field.cells[48..64].iter().all(Option::is_none));
        let high = field
            .decode_mbaff(
                3,
                [0, 0],
                [4, 4],
                8,
                Partition::Median,
                [Some(62), Some(63)],
                [[2, -3], [-4, 5]],
                mode,
            )
            .unwrap();
        assert_eq!(
            high,
            [
                Neighbour::Inter {
                    reference: 62,
                    vector: [2, -3]
                },
                Neighbour::Inter {
                    reference: 63,
                    vector: [-4, 5]
                },
            ]
        );
    }
    #[test]
    fn mbaff_skip_derives_from_normalized_cells_and_publishes_full_block() {
        let mut field = MotionField::new(32, 64, 65536).unwrap();
        for (address, vector) in [(1, [12, 13]), (3, [8, 9]), (4, [4, -5])] {
            field
                .store_mbaff(
                    address,
                    [0, 0],
                    [16, 16],
                    7,
                    false,
                    [
                        Neighbour::Inter {
                            reference: 0,
                            vector,
                        },
                        Neighbour::NoPrediction,
                    ],
                )
                .unwrap();
        }
        assert_eq!(
            field.decode_p_skip_mbaff(6, 7, |p| Some(p == 3)).unwrap(),
            [8, 4]
        );
        for cell in &field.cells[96..112] {
            assert_eq!(
                cell.unwrap().lists,
                [
                    Neighbour::Inter {
                        reference: 0,
                        vector: [8, 4]
                    },
                    Neighbour::NoPrediction
                ]
            );
            assert!(cell.unwrap().field);
        }
        assert!(field.decode_p_skip_mbaff(6, 7, |p| Some(p == 3)).is_err());
        assert!(field.decode_p_skip_mbaff(7, 7, |_| None).is_err());
        assert!(field.cells[112..128].iter().all(Option::is_none));
    }
    #[test]
    fn mbaff_publication_normalizes_mixed_neighbours_and_preserves_slice_boundaries() {
        let motion = |reference, vector| {
            [
                Neighbour::Inter { reference, vector },
                Neighbour::NoPrediction,
            ]
        };
        let mut field = MotionField::new(32, 32, 65536).unwrap();
        field
            .store_mbaff(0, [0, 0], [16, 16], 7, false, motion(1, [3, -5]))
            .unwrap();
        field
            .store_mbaff(1, [0, 0], [16, 16], 7, false, motion(1, [9, -7]))
            .unwrap();
        let neighbours = field
            .neighbours_mbaff(2, [0, 8], [8, 8], 7, 0, |p| Some(p == 1))
            .unwrap();
        assert_eq!(neighbours.left, motion(2, [9, -3])[0]);
        assert_eq!(neighbours.top_left, motion(2, [3, -2])[0]);
        assert_eq!(neighbours.top, Neighbour::Unavailable);
        assert_eq!(
            super::super::avc_mv::predict_for_field(2, Partition::Median, neighbours, true)
                .unwrap(),
            [3, -2] // C falls back to D; B contributes the unavailable zero vector.
        );
        let other_slice = field
            .neighbours_mbaff(2, [0, 8], [8, 8], 8, 0, |p| Some(p == 1))
            .unwrap();
        assert_eq!(other_slice.left, Neighbour::Unavailable);
        let unused = field
            .neighbours_mbaff(2, [0, 8], [8, 8], 7, 1, |p| Some(p == 1))
            .unwrap();
        assert_eq!(unused.left, Neighbour::NoPrediction);
        field
            .store_mbaff(2, [0, 0], [16, 16], 7, true, motion(63, [7, -7]))
            .unwrap();
        assert!(
            field
                .store_mbaff(3, [0, 0], [16, 16], 7, false, motion(0, [0; 2]))
                .is_err()
        );
        assert!(field.snapshot([&[1], &[]], 65536).is_err());
        assert!(field.neighbours([16, 0], [16, 16], 7, 0).is_err());
        assert!(
            field
                .store_mbaff(3, [usize::MAX, 0], [4, 4], 7, true, motion(0, [0; 2]))
                .is_err()
        );
        let mut reverse = MotionField::new(32, 32, 65536).unwrap();
        reverse
            .store_mbaff(0, [0, 0], [16, 16], 7, true, motion(63, [6, -7]))
            .unwrap();
        let n = reverse
            .neighbours_mbaff(2, [0, 0], [16, 16], 7, 0, |p| Some(p == 0))
            .unwrap();
        assert_eq!(n.left, motion(31, [6, -14])[0]);
    }
    #[test]
    fn address_owned_motion_keeps_raster_colocated_cells_and_slice_identity() {
        let mut field = MotionField::new(32, 32, 65536).unwrap();
        for y in (0..32).step_by(4) {
            for x in (0..32).step_by(4) {
                field
                    .store(
                        [x, y],
                        [4, 4],
                        (y / 16 * 2 + x / 16) as u32,
                        [
                            Neighbour::Inter {
                                reference: 0,
                                vector: [x as i16, y as i16],
                            },
                            Neighbour::NoPrediction,
                        ],
                    )
                    .unwrap();
            }
        }
        assert!(matches!(
            field.cells[16].unwrap().lists[0],
            Neighbour::Inter {
                vector: [16, 0],
                ..
            }
        ));
        assert!(matches!(
            field.cells[32].unwrap().lists[0],
            Neighbour::Inter {
                vector: [0, 16],
                ..
            }
        ));
        let ids = [100u64, 200, 300, 400];
        let mappings: Vec<_> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (i as u32, [std::slice::from_ref(id), &[][..]]))
            .collect();
        let saved = field.snapshot_slices(&mappings, 65536).unwrap();
        for y in (0..32).step_by(4) {
            for x in (0..32).step_by(4) {
                let stored = saved.at([x, y]).unwrap()[0].unwrap();
                assert_eq!(stored.picture_id, ids[y / 16 * 2 + x / 16]);
                assert_eq!(stored.vector, [x as i16, y as i16]);
            }
        }
    }
    #[test]
    fn independent_slice_lists_resolve_the_same_index_to_different_pictures() {
        let mut field = MotionField::new(32, 16, 65536).unwrap();
        for (x, id) in [(0, 7), (16, 11)] {
            field
                .store(
                    [x, 0],
                    [16, 16],
                    id,
                    [
                        Neighbour::Inter {
                            reference: 0,
                            vector: [4, -2],
                        },
                        Neighbour::NoPrediction,
                    ],
                )
                .unwrap();
        }
        let mappings = [
            (7, [&[100u64][..], &[][..]]),
            (11, [&[200u64][..], &[][..]]),
        ];
        let saved = field.snapshot_slices(&mappings, 65536).unwrap();
        assert_eq!(saved.at([0, 0]).unwrap()[0].unwrap().picture_id, 100);
        assert_eq!(saved.at([16, 0]).unwrap()[0].unwrap().picture_id, 200);
        assert_eq!(saved.at([16, 0]).unwrap()[0].unwrap().vector, [4, -2]);
        assert!(field.snapshot_slices(&mappings[..1], 65536).is_err());
        assert!(
            field
                .snapshot_slices(&[mappings[0], mappings[0]], 65536)
                .is_err()
        );
        assert!(field.snapshot_slices(&mappings, 1).is_err());
        assert!(field.snapshot([&[100], &[]], 65536).is_err());
    }
}
