//! Slice-aware CAVLC coefficient-count storage for progressive 4:2:0 pictures.
use super::{
    avc_inter::{InterHeader, InterSyntax, read_inter_header},
    avc_inter_coefficients::{CoefficientNeighbours, InterCoefficients, read_inter_coefficients},
    bits::BitReader,
};
use crate::{Result, invalid};
#[derive(Clone, Copy)]
struct Counts {
    slice: u32,
    luma: [u8; 16],
    chroma: [[u8; 4]; 2],
}
pub struct CoefficientField {
    width: usize,
    cells: Vec<Option<Counts>>,
}
impl CoefficientField {
    pub fn new(width_mbs: usize, height_mbs: usize, memory_limit: usize) -> Result<Self> {
        let count = width_mbs
            .checked_mul(height_mbs)
            .filter(|&n| n > 0)
            .ok_or_else(|| invalid("invalid AVC coefficient-field geometry"))?;
        if count
            .checked_mul(std::mem::size_of::<Option<Counts>>())
            .is_none_or(|n| n > memory_limit)
        {
            return Err(invalid("AVC coefficient field exceeds memory budget"));
        }
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate AVC coefficient field"))?;
        cells.resize(count, None);
        Ok(Self {
            width: width_mbs,
            cells,
        })
    }
    pub fn neighbours(&self, address: usize, slice: u32) -> Result<CoefficientNeighbours> {
        if address >= self.cells.len() {
            return Err(invalid("AVC macroblock address out of range"));
        }
        let left = if address % self.width == 0 {
            None
        } else {
            self.cells[address - 1]
        }
        .filter(|c| c.slice == slice);
        let top = address
            .checked_sub(self.width)
            .and_then(|n| self.cells[n])
            .filter(|c| c.slice == slice);
        Ok(CoefficientNeighbours {
            luma_left: std::array::from_fn(|y| left.map(|c| c.luma[y * 4 + 3])),
            luma_top: std::array::from_fn(|x| top.map(|c| c.luma[12 + x])),
            chroma_left: std::array::from_fn(|p| {
                std::array::from_fn(|y| left.map(|c| c.chroma[p][y * 2 + 1]))
            }),
            chroma_top: std::array::from_fn(|p| {
                std::array::from_fn(|x| top.map(|c| c.chroma[p][2 + x]))
            }),
        })
    }
    /// Also used by skipped (zero counts), intra and PCM (16 counts) macroblocks.
    pub fn store(
        &mut self,
        address: usize,
        slice: u32,
        luma: [u8; 16],
        chroma: [[u8; 4]; 2],
    ) -> Result<()> {
        if luma.iter().chain(chroma.iter().flatten()).any(|&n| n > 16) {
            return Err(invalid("AVC coefficient count exceeds block size"));
        }
        let cell = self
            .cells
            .get_mut(address)
            .ok_or_else(|| invalid("AVC macroblock address out of range"))?;
        if cell.is_some() {
            return Err(invalid("AVC coefficient macroblock already decoded"));
        }
        *cell = Some(Counts {
            slice,
            luma,
            chroma,
        });
        Ok(())
    }
    /// Parse a complete non-skipped inter macroblock through its coefficients.
    /// The bit cursor and stored counts are committed together on success.
    pub fn read_inter(
        &mut self,
        bits: &mut BitReader<'_>,
        address: usize,
        slice: u32,
        syntax: &InterSyntax,
    ) -> Result<(InterHeader, InterCoefficients)> {
        if syntax.chroma_array_type != 1 {
            return Err(invalid("inter coefficient field requires 4:2:0"));
        }
        let neighbours = self.neighbours(address, slice)?;
        if self.cells[address].is_some() {
            return Err(invalid("AVC coefficient macroblock already decoded"));
        }
        let mut input = bits.clone();
        let header = read_inter_header(&mut input, syntax)?;
        let coefficients = read_inter_coefficients(
            &mut input,
            header.residual.pattern,
            header.residual.transform8,
            neighbours,
        )?;
        self.store(
            address,
            slice,
            coefficients.luma_counts,
            coefficients.chroma_counts,
        )?;
        *bits = input;
        Ok((header, coefficients))
    }
}
#[cfg(test)]
mod tests {
    use super::super::avc_slice::SliceType;
    use super::*;
    #[test]
    fn complete_inter_macroblock_and_neighbour_boundaries() {
        let mut field = CoefficientField::new(2, 2, 8192).unwrap();
        let syntax = InterSyntax {
            slice: SliceType::P,
            active_references: [1, 0],
            previous_qp: 26,
            bit_depth: 8,
            chroma_array_type: 1,
            transform8_enabled: false,
            direct8_inference: true,
        };
        // mb_type=0, MVD=(0,0), CBP=0: four one bits, no residual syntax.
        let mut bits = BitReader::new(&[0xf0]);
        let (h, c) = field.read_inter(&mut bits, 0, 7, &syntax).unwrap();
        assert_eq!(bits.position(), 4);
        assert_eq!(h.residual.qp, 26);
        assert_eq!(c.luma_counts, [0; 16]);
        assert_eq!(field.neighbours(1, 7).unwrap().luma_left, [Some(0); 4]);
        assert_eq!(field.neighbours(2, 7).unwrap().luma_top, [Some(0); 4]);
        assert_eq!(field.neighbours(1, 8).unwrap().luma_left, [None; 4]);
        let mut bits = BitReader::new(&[0xe0]);
        assert!(field.read_inter(&mut bits, 1, 7, &syntax).is_err());
        assert_eq!(bits.position(), 0);
        field.store(1, 7, [16; 16], [[16; 4]; 2]).unwrap();
        assert_eq!(
            field.neighbours(3, 7).unwrap().chroma_top,
            [[Some(16); 2]; 2]
        );
        assert!(CoefficientField::new(2, 2, 1).is_err());
    }
}
