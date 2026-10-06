//! Address-owned CABAC reference-index/MVD contexts with explicit MBAFF P APIs.
use super::{
    avc_cabac_inter::{self as syntax, InterBins},
    avc_inter::{self, MacroblockType, Partition, Prediction},
    avc_slice::SliceType,
};
use crate::{Result, invalid};
#[derive(Clone, Copy)]
struct Cell {
    slice: u32,
    references: [Option<u8>; 2],
    magnitudes: [[u32; 2]; 2],
    field: bool,
}
pub struct CabacMotionContexts {
    width: usize,
    height: usize,
    cells: Vec<Option<Cell>>,
    failed: bool,
    mbaff: bool,
}
// Cells are owned by macroblock address, then its local 4x4 raster index.
// Progressive spatial queries translate coordinates at the storage boundary.
fn cell_index(width: usize, x: usize, y: usize) -> usize {
    (y / 4 * (width / 4) + x / 4) * 16 + y % 4 * 4 + x % 4
}
impl CabacMotionContexts {
    pub fn new(width_mbs: usize, height_mbs: usize, budget: usize) -> Result<Self> {
        let count = width_mbs
            .checked_mul(height_mbs)
            .filter(|n| *n > 0 && *n <= 65536)
            .and_then(|n| n.checked_mul(16))
            .ok_or_else(|| invalid("CABAC motion geometry exceeds limits"))?;
        if count
            .checked_mul(std::mem::size_of::<Option<Cell>>())
            .is_none_or(|n| n > budget)
        {
            return Err(invalid("CABAC motion context budget exceeded"));
        }
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(count)
            .map_err(|_| invalid("CABAC motion context allocation failed"))?;
        cells.resize(count, None);
        Ok(Self {
            width: width_mbs * 4,
            height: height_mbs * 4,
            cells,
            failed: false,
            mbaff: false,
        })
    }
    /// MBAFF address-owned CABAC contexts; height is the coded macroblock count.
    pub fn new_mbaff(width_mbs: usize, height_mbs: usize, budget: usize) -> Result<Self> {
        if height_mbs % 2 != 0 {
            return Err(invalid("MBAFF CABAC motion needs complete pairs"));
        }
        let mut result = Self::new(width_mbs, height_mbs, budget)?;
        result.mbaff = true;
        Ok(result)
    }
    fn check_pair(&self, address: usize, field: bool) -> Result<()> {
        if !self.mbaff {
            return Err(invalid("CABAC motion is not MBAFF"));
        }
        if self.cells[address / 2 * 32..address / 2 * 32 + 32]
            .iter()
            .flatten()
            .any(|c| c.field != field)
        {
            return Err(invalid("CABAC motion pair mode changed"));
        }
        Ok(())
    }
    fn origin(&self, address: usize) -> Result<[usize; 2]> {
        if self.failed {
            return Err(invalid("CABAC motion contexts previously failed"));
        }
        if address >= self.width / 4 * (self.height / 4) {
            return Err(invalid("CABAC motion address out of range"));
        }
        let origin = [
            address % (self.width / 4) * 4,
            address / (self.width / 4) * 4,
        ];
        for y in 0..4 {
            for x in 0..4 {
                if self.cells[cell_index(self.width, origin[0] + x, origin[1] + y)].is_some() {
                    return Err(invalid("CABAC motion macroblock already stored"));
                }
            }
        }
        Ok(origin)
    }
    fn neighbours(&self, x: usize, y: usize, slice: u32) -> [Option<Cell>; 2] {
        [
            if x > 0 {
                self.cells[cell_index(self.width, x - 1, y)]
            } else {
                None
            },
            if y > 0 {
                self.cells[cell_index(self.width, x, y - 1)]
            } else {
                None
            },
        ]
        .map(|cell| cell.filter(|v| v.slice == slice))
    }
    fn fill(&mut self, origin: [usize; 2], slice: u32, field: bool) {
        for y in 0..4 {
            for x in 0..4 {
                self.cells[cell_index(self.width, origin[0] + x, origin[1] + y)] = Some(Cell {
                    slice,
                    references: [None; 2],
                    magnitudes: [[0; 2]; 2],
                    field,
                });
            }
        }
    }
    /// Intra/skip neighbours contribute zero to reference and MVD contexts.
    pub fn store_non_inter(&mut self, address: usize, slice: u32) -> Result<()> {
        if self.mbaff {
            return Err(invalid("MBAFF CABAC motion needs pair-aware publication"));
        }
        let origin = self.origin(address)?;
        self.fill(origin, slice, false);
        Ok(())
    }
    pub fn store_non_inter_mbaff(&mut self, address: usize, slice: u32, field: bool) -> Result<()> {
        let origin = self.origin(address)?;
        self.check_pair(address, field)?;
        self.fill(origin, slice, field);
        Ok(())
    }
    fn context_neighbours(
        &self,
        address: usize,
        local: [u8; 2],
        slice: u32,
        field: bool,
        pair_field: &mut dyn FnMut(usize) -> Option<bool>,
    ) -> Result<[Option<Cell>; 2]> {
        if !self.mbaff {
            let origin = [
                address % (self.width / 4) * 4,
                address / (self.width / 4) * 4,
            ];
            return Ok(self.neighbours(
                origin[0] + usize::from(local[0]) / 4,
                origin[1] + usize::from(local[1]) / 4,
                slice,
            ));
        }
        if pair_field(address / 2) != Some(field) {
            return Err(invalid("CABAC motion current pair mode unavailable"));
        }
        let spatial = super::avc_mbaff::motion_neighbours(
            address,
            local.map(usize::from),
            [4, 4],
            self.width / 4,
            self.height / 4,
            true,
            &mut *pair_field,
        )?;
        let mut output = [None; 2];
        for i in 0..2 {
            if let Some((address, local)) = spatial[i] {
                if let Some(mut cell) =
                    self.cells[address * 16 + local[1] * 4 + local[0]].filter(|c| c.slice == slice)
                {
                    if pair_field(address / 2) != Some(cell.field) {
                        return Err(invalid("CABAC neighbour pair mode mismatch"));
                    }
                    // H.264 9.3.3.1.1.6: a frame block treats field ref 0/1 as zero.
                    if !field && cell.field {
                        cell.references = cell.references.map(|r| r.map(|r| r / 2));
                    }
                    for magnitude in &mut cell.magnitudes {
                        magnitude[1] = match (field, cell.field) {
                            (false, true) => magnitude[1]
                                .checked_mul(2)
                                .ok_or_else(|| invalid("CABAC MVD magnitude overflow"))?,
                            (true, false) => magnitude[1] / 2,
                            _ => magnitude[1],
                        };
                    }
                    output[i] = Some(cell);
                }
            }
        }
        Ok(output)
    }
    /// mb_type has already been decoded. Reads subtypes, then references for
    /// both lists, then MVDs for both lists in syntax order. Errors poison the
    /// context grid along with the caller's arithmetic decoding state.
    pub fn read_prediction(
        &mut self,
        bins: &mut impl InterBins,
        address: usize,
        slice_id: u32,
        slice: SliceType,
        code: u8,
        active: [u32; 2],
    ) -> Result<Vec<Partition>> {
        if self.mbaff {
            return Err(invalid("MBAFF CABAC prediction needs pair-aware dispatch"));
        }
        let result = self.read_inner(
            bins,
            address,
            slice_id,
            slice,
            code,
            active,
            false,
            &mut |_| None,
        );
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub fn read_prediction_mbaff(
        &mut self,
        bins: &mut impl InterBins,
        address: usize,
        slice: u32,
        code: u8,
        active: [u32; 2],
        field: bool,
        mut pair_field: impl FnMut(usize) -> Option<bool>,
    ) -> Result<Vec<Partition>> {
        if !self.mbaff {
            return Err(invalid("CABAC prediction context is not MBAFF"));
        }
        let result = self.read_inner(
            bins,
            address,
            slice,
            SliceType::P,
            code,
            active,
            field,
            &mut pair_field,
        );
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn read_inner(
        &mut self,
        bins: &mut impl InterBins,
        address: usize,
        slice_id: u32,
        slice: SliceType,
        code: u8,
        active: [u32; 2],
        field: bool,
        pair_field: &mut dyn FnMut(usize) -> Option<bool>,
    ) -> Result<Vec<Partition>> {
        let origin = self.origin(address)?;
        if self.mbaff {
            self.check_pair(address, field)?;
        }
        if !matches!(slice, SliceType::P | SliceType::B)
            || active[0] == 0
            || active[0] > if field { 64 } else { 32 }
            || (slice == SliceType::B
                && (active[1] == 0 || active[1] > if field { 64 } else { 32 }))
        {
            return Err(invalid("invalid CABAC prediction slice/reference count"));
        }
        let mut parts = match avc_inter::macroblock_type(slice, u32::from(code))? {
            MacroblockType::Inter {
                partitions,
                reference_zero: false,
            } => partitions,
            MacroblockType::Subdivided {
                reference_zero: false,
            } => {
                let mut sub = [0u32; 4];
                for code in &mut sub {
                    *code = u32::from(syntax::sub_macroblock_type(bins, slice)?);
                }
                avc_inter::sub_partitions(slice, sub)?
            }
            _ => {
                return Err(invalid(
                    "CABAC inter prediction requires a non-intra, non-P_8x8ref0 type",
                ));
            }
        };
        self.fill(origin, slice_id, field);
        for list in 0..if slice == SliceType::B { 2 } else { 1 } {
            for group in 0..4 {
                let Some(first) = parts
                    .iter()
                    .find(|p| p.group == group && uses(p.prediction, list))
                else {
                    continue;
                };
                let neighbours = self
                    .context_neighbours(address, first.origin, slice_id, field, pair_field)?
                    .map(|v| v.and_then(|v| v.references[list]).is_some_and(|r| r > 0));
                let reference =
                    syntax::reference_index_for_field(bins, neighbours, active[list], field)?;
                for part in parts
                    .iter_mut()
                    .filter(|p| p.group == group && uses(p.prediction, list))
                {
                    part.references[list] = Some(reference);
                    self.update(origin, part, |cell| cell.references[list] = Some(reference));
                }
            }
        }
        for list in 0..if slice == SliceType::B { 2 } else { 1 } {
            for part in parts.iter_mut().filter(|p| uses(p.prediction, list)) {
                let neighbours =
                    self.context_neighbours(address, part.origin, slice_id, field, pair_field)?;
                for component in 0..2 {
                    let magnitudes =
                        neighbours.map(|v| v.map_or(0, |v| v.magnitudes[list][component]));
                    part.differences[list][component] =
                        syntax::motion_difference(bins, component, magnitudes)?;
                }
                let magnitudes = part.differences[list].map(i32::unsigned_abs);
                self.update(origin, part, |cell| cell.magnitudes[list] = magnitudes);
            }
        }
        Ok(parts)
    }
    fn update(&mut self, origin: [usize; 2], part: &Partition, mut change: impl FnMut(&mut Cell)) {
        for y in 0..usize::from(part.size[1]) / 4 {
            for x in 0..usize::from(part.size[0]) / 4 {
                let at = cell_index(
                    self.width,
                    origin[0] + usize::from(part.origin[0]) / 4 + x,
                    origin[1] + usize::from(part.origin[1]) / 4 + y,
                );
                if let Some(cell) = &mut self.cells[at] {
                    change(cell);
                }
            }
        }
    }
}
fn uses(prediction: Prediction, list: usize) -> bool {
    prediction == Prediction::Bi
        || prediction
            == if list == 0 {
                Prediction::L0
            } else {
                Prediction::L1
            }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Bins(VecDeque<(i32, bool)>);
    impl Bins {
        fn new(values: &[(i32, bool)]) -> Self {
            Self(values.iter().copied().collect())
        }
        fn read(&mut self, context: i32) -> Result<bool> {
            let (expected, value) = self
                .0
                .pop_front()
                .ok_or_else(|| invalid("test entropy exhausted"))?;
            assert_eq!(expected, context);
            Ok(value)
        }
        fn done(self) {
            assert!(self.0.is_empty());
        }
    }
    impl InterBins for Bins {
        fn decision(&mut self, c: usize) -> Result<bool> {
            self.read(c as i32)
        }
        fn bypass(&mut self) -> Result<bool> {
            self.read(-1)
        }
        fn terminate(&mut self) -> Result<bool> {
            self.read(-2)
        }
    }
    fn first(grid: &mut CabacMotionContexts) {
        // P_8x16: decode both ref_idx values before either partition's MVD.
        // Right ref context sees left ref=1; right MVD context sees left dx=3.
        let mut bins = Bins::new(&[
            (54, true),
            (58, false),
            (55, true),
            (58, false),
            (40, true),
            (43, true),
            (44, true),
            (45, false),
            (-1, false),
            (47, false),
            (41, false),
            (47, false),
        ]);
        let parts = grid
            .read_prediction(&mut bins, 0, 7, SliceType::P, 2, [2, 0])
            .unwrap();
        bins.done();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].differences[0], [3, 0]);
        assert_eq!(parts[1].differences[0], [0, 0]);
        assert!(parts.iter().all(|p| p.references == [Some(1), None]));
    }
    #[test]
    fn mbaff_reference_conditions_and_vertical_mvd_use_normative_units() {
        for (source_field, current_field, reference, magnitude, contexts) in [
            (true, false, 1, 2, [54, 41, 48]),
            (true, false, 2, 2, [55, 41, 48]),
            (false, true, 1, 5, [55, 41, 47]),
            (true, true, 1, 2, [55, 41, 47]),
            (false, false, 1, 5, [55, 41, 48]),
        ] {
            let mut grid = CabacMotionContexts::new_mbaff(2, 2, 65536).unwrap();
            for address in 0..2 {
                grid.store_non_inter_mbaff(address, 7, source_field)
                    .unwrap();
                let origin = [
                    address % (grid.width / 4) * 4,
                    address / (grid.width / 4) * 4,
                ];
                let part = Partition {
                    origin: [0, 0],
                    size: [16, 16],
                    prediction: Prediction::L0,
                    group: 0,
                    references: [Some(reference), None],
                    differences: [[0; 2]; 2],
                };
                grid.update(origin, &part, |cell| {
                    cell.references[0] = Some(reference);
                    cell.magnitudes[0] = [3, magnitude];
                });
            }
            let mut bins = Bins::new(&contexts.map(|ctx| (ctx, false)));
            let parts = grid
                .read_prediction_mbaff(&mut bins, 2, 7, 0, [2, 0], current_field, |pair| {
                    Some(if pair == 0 {
                        source_field
                    } else {
                        current_field
                    })
                })
                .unwrap();
            bins.done();
            assert_eq!(parts[0].references, [Some(0), None]);
            assert_eq!(parts[0].differences, [[0; 2]; 2]);
        }
    }
    #[test]
    fn mbaff_expanded_list_slice_isolation_and_error_poisoning() {
        let mut grid = CabacMotionContexts::new_mbaff(1, 2, 65536).unwrap();
        let mut decisions = vec![(54, true), (58, true)];
        decisions.extend(std::iter::repeat_n((59, true), 61));
        decisions.extend([(59, false), (40, false), (47, false)]);
        let mut bins = Bins::new(&decisions);
        let parts = grid
            .read_prediction_mbaff(&mut bins, 0, 3, 0, [64, 0], true, |_| Some(true))
            .unwrap();
        assert_eq!(parts[0].references, [Some(63), None]);
        bins.done();
        let mut bins = Bins::new(&[(54, false), (40, false), (47, false)]);
        grid.read_prediction_mbaff(&mut bins, 1, 4, 0, [2, 0], true, |_| Some(true))
            .unwrap();
        bins.done();
        assert!(grid.store_non_inter_mbaff(0, 3, false).is_err());
        let mut grid = CabacMotionContexts::new_mbaff(1, 2, 65536).unwrap();
        let mut bins = Bins::new(&[]);
        assert!(
            grid.read_prediction_mbaff(&mut bins, 0, 3, 0, [2, 0], true, |_| None)
                .is_err()
        );
        assert!(grid.store_non_inter_mbaff(1, 3, true).is_err());
        let mut pair = CabacMotionContexts::new_mbaff(1, 2, 65536).unwrap();
        pair.store_non_inter_mbaff(0, 3, true).unwrap();
        assert!(pair.store_non_inter_mbaff(1, 3, false).is_err());
        assert!(pair.cells[16..32].iter().all(Option::is_none));
        let mut isolated = CabacMotionContexts::new_mbaff(2, 2, 65536).unwrap();
        for address in 0..2 {
            isolated.store_non_inter_mbaff(address, 7, true).unwrap();
            for cell in &mut isolated.cells[address * 16..address * 16 + 16] {
                let cell = cell.as_mut().unwrap();
                cell.references[0] = Some(2);
                cell.magnitudes[0] = [3, 2];
            }
        }
        let mut bins = Bins::new(&[(54, false), (40, false), (47, false)]);
        isolated
            .read_prediction_mbaff(&mut bins, 2, 8, 0, [2, 0], false, |p| Some(p == 0))
            .unwrap();
        bins.done();
        assert!(CabacMotionContexts::new_mbaff(1, 1, 65536).is_err());
        assert!(CabacMotionContexts::new_mbaff(1, 2, 1).is_err());
    }
    #[test]
    fn address_owned_cells_keep_distinct_partition_contexts_and_spatial_neighbours() {
        let mut grid = CabacMotionContexts::new(2, 2, 65536).unwrap();
        for address in 0..4 {
            let origin = grid.origin(address).unwrap();
            grid.fill(origin, 7, false);
            for y in 0..4 {
                for x in 0..4 {
                    let part = Partition {
                        origin: [(x * 4) as u8, (y * 4) as u8],
                        size: [4, 4],
                        prediction: Prediction::L0,
                        group: 0,
                        references: [Some(0), None],
                        differences: [[0; 2]; 2],
                    };
                    grid.update(origin, &part, |cell| {
                        cell.magnitudes[0] = [(address * 16 + y * 4 + x) as u32, 0]
                    });
                }
            }
        }
        for index in 0..64 {
            assert_eq!(grid.cells[index].unwrap().magnitudes[0][0], index as u32);
        }
        let neighbours = grid.neighbours(4, 4, 7);
        assert_eq!(neighbours[0].unwrap().magnitudes[0][0], 35);
        assert_eq!(neighbours[1].unwrap().magnitudes[0][0], 28);
        assert!(grid.neighbours(4, 4, 8).iter().all(Option::is_none));
    }
    #[test]
    fn partition_updates_cross_macroblocks_but_not_slice_boundaries() {
        for (slice, context) in [(7, 55), (8, 54)] {
            let mut grid = CabacMotionContexts::new(2, 1, 65536).unwrap();
            first(&mut grid);
            let mut bins = Bins::new(&[(context, false), (40, false), (47, false)]);
            let parts = grid
                .read_prediction(&mut bins, 1, slice, SliceType::P, 0, [2, 0])
                .unwrap();
            assert_eq!(parts[0].references, [Some(0), None]);
            bins.done();
        }
        let mut grid = CabacMotionContexts::new(2, 2, 65536).unwrap();
        first(&mut grid);
        let mut bins = Bins::new(&[(56, false), (41, false), (47, false)]);
        grid.read_prediction(&mut bins, 2, 7, SliceType::P, 0, [2, 0])
            .unwrap();
        bins.done();
    }
    #[test]
    fn direct_and_non_inter_neighbours_supply_zero_conditions() {
        let mut grid = CabacMotionContexts::new(3, 1, 65536).unwrap();
        grid.store_non_inter(0, 0).unwrap();
        let mut bins = Bins::new(&[]);
        let parts = grid
            .read_prediction(&mut bins, 1, 0, SliceType::B, 0, [2, 2])
            .unwrap();
        assert!(parts.iter().all(|p| p.references == [None; 2]));
        bins.done();
        // B_Bi_16x16, references first for both lists; then L0 and L1 MVDs.
        let mut bins = Bins::new(&[
            (54, false),
            (54, false),
            (40, false),
            (47, false),
            (40, false),
            (47, false),
        ]);
        let parts = grid
            .read_prediction(&mut bins, 2, 0, SliceType::B, 3, [2, 2])
            .unwrap();
        assert_eq!(parts[0].references, [Some(0), Some(0)]);
        bins.done();
    }
    #[test]
    fn budgets_overwrites_and_failed_entropy_do_not_allow_reuse() {
        assert!(CabacMotionContexts::new(usize::MAX, 2, usize::MAX).is_err());
        assert!(CabacMotionContexts::new(1, 1, 0).is_err());
        let mut grid = CabacMotionContexts::new(1, 1, 65536).unwrap();
        let mut bins = Bins::new(&[]);
        assert!(
            grid.read_prediction(&mut bins, 0, 0, SliceType::P, 0, [1, 0])
                .is_err()
        );
        assert!(grid.store_non_inter(0, 0).is_err());
        let mut grid = CabacMotionContexts::new(1, 1, 65536).unwrap();
        grid.store_non_inter(0, 0).unwrap();
        assert!(grid.store_non_inter(0, 0).is_err());
    }
}
