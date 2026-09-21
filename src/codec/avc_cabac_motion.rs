//! Spatial CABAC reference-index/MVD contexts for progressive frame slices.
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
}
pub struct CabacMotionContexts {
    width: usize,
    height: usize,
    cells: Vec<Option<Cell>>,
    failed: bool,
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
        })
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
                if self.cells[(origin[1] + y) * self.width + origin[0] + x].is_some() {
                    return Err(invalid("CABAC motion macroblock already stored"));
                }
            }
        }
        Ok(origin)
    }
    fn neighbours(&self, x: usize, y: usize, slice: u32) -> [Option<Cell>; 2] {
        [
            if x > 0 {
                self.cells[y * self.width + x - 1]
            } else {
                None
            },
            if y > 0 {
                self.cells[(y - 1) * self.width + x]
            } else {
                None
            },
        ]
        .map(|cell| cell.filter(|v| v.slice == slice))
    }
    fn fill(&mut self, origin: [usize; 2], slice: u32) {
        for y in 0..4 {
            for x in 0..4 {
                self.cells[(origin[1] + y) * self.width + origin[0] + x] = Some(Cell {
                    slice,
                    references: [None; 2],
                    magnitudes: [[0; 2]; 2],
                });
            }
        }
    }
    /// Intra/skip neighbours contribute zero to reference and MVD contexts.
    pub fn store_non_inter(&mut self, address: usize, slice: u32) -> Result<()> {
        let origin = self.origin(address)?;
        self.fill(origin, slice);
        Ok(())
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
        let result = self.read_inner(bins, address, slice_id, slice, code, active);
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
    ) -> Result<Vec<Partition>> {
        let origin = self.origin(address)?;
        if !matches!(slice, SliceType::P | SliceType::B)
            || active[0] == 0
            || active[0] > 32
            || (slice == SliceType::B && (active[1] == 0 || active[1] > 32))
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
        self.fill(origin, slice_id);
        for list in 0..if slice == SliceType::B { 2 } else { 1 } {
            for group in 0..4 {
                let Some(first) = parts
                    .iter()
                    .find(|p| p.group == group && uses(p.prediction, list))
                else {
                    continue;
                };
                let [x, y] = [
                    origin[0] + usize::from(first.origin[0]) / 4,
                    origin[1] + usize::from(first.origin[1]) / 4,
                ];
                let neighbours = self
                    .neighbours(x, y, slice_id)
                    .map(|v| v.and_then(|v| v.references[list]).is_some_and(|r| r > 0));
                let reference = syntax::reference_index(bins, neighbours, active[list])?;
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
                let [x, y] = [
                    origin[0] + usize::from(part.origin[0]) / 4,
                    origin[1] + usize::from(part.origin[1]) / 4,
                ];
                let neighbours = self.neighbours(x, y, slice_id);
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
                let at = (origin[1] + usize::from(part.origin[1]) / 4 + y) * self.width
                    + origin[0]
                    + usize::from(part.origin[0]) / 4
                    + x;
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
