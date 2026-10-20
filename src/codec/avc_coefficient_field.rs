//! Slice-aware CAVLC coefficient-count storage with frame/pair neighbour APIs.
use super::{
    avc_inter::{InterHeader, InterSyntax},
    avc_inter_coefficients::{CoefficientNeighbours, InterCoefficients},
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
    chroma422:Option<Vec<Option<[[u8;8];2]>>>,
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
            chroma422:None,
        })
    }
    pub(super) fn new_422(width:usize,height:usize,limit:usize)->Result<Self> {
        let mut field=Self::new(width,height,limit)?;
        let count=field.cells.len();let base=field.cells.capacity().checked_mul(std::mem::size_of::<Option<Counts>>()).ok_or_else(||invalid("422 coefficient storage overflow"))?;
        if count.checked_mul(std::mem::size_of::<Option<[[u8;8];2]>>()).and_then(|n|n.checked_add(base)).is_none_or(|n|n>limit){return Err(invalid("422 coefficient field exceeds memory budget"));}
        let mut wide=Vec::new();wide.try_reserve_exact(count).map_err(|_|invalid("cannot allocate 422 coefficient field"))?;wide.resize(count,None);
        if wide.capacity().checked_mul(std::mem::size_of::<Option<[[u8;8];2]>>()).and_then(|n|n.checked_add(base)).is_none_or(|n|n>limit){return Err(invalid("422 coefficient capacities exceed memory budget"));}
        field.chroma422=Some(wide);Ok(field)
    }
    pub(super) fn neighbours_422(&self,address:usize,slice:u32)->Result<super::avc_inter_coefficients::Chroma422Neighbours> {
        if address>=self.cells.len(){return Err(invalid("422 coefficient address out of range"));}
        let wide=self.chroma422.as_ref().ok_or_else(||invalid("missing 422 coefficient contexts"))?;
        let value=|owner:Option<usize>,component:usize,cell:usize|owner.and_then(|i|self.cells[i].filter(|c|c.slice==slice).and_then(|_|wide[i]).map(|c|c[component][cell]));
        let left=(address%self.width!=0).then(||address-1);let top=address.checked_sub(self.width);
        Ok(super::avc_inter_coefficients::Chroma422Neighbours {left:std::array::from_fn(|p|std::array::from_fn(|y|value(left,p,y*2+1))),top:std::array::from_fn(|p|std::array::from_fn(|x|value(top,p,6+x)))})
    }
    fn neighbours_422_mbaff(&self,address:usize,slice:u32,mut pair_field:impl FnMut(usize)->Option<bool>)->Result<super::avc_inter_coefficients::Chroma422Neighbours> {
        if address>=self.cells.len(){return Err(invalid("422 MBAFF coefficient address out of range"));}
        let wide=self.chroma422.as_ref().ok_or_else(||invalid("missing 422 MBAFF coefficient contexts"))?;
        let mut out=super::avc_inter_coefficients::Chroma422Neighbours::default();
        for component in 0..2 {
            for row in 0..4 {
                let neighbour=super::avc_mbaff::neighbour_location(address,[-1,(row*4) as isize],self.width,self.cells.len()/self.width,true,[2,1],&mut pair_field)?;
                out.left[component][row]=neighbour.and_then(|(owner,local)|self.cells[owner].filter(|c|c.slice==slice).and_then(|_|wide[owner]).map(|counts|counts[component][local[1]/4*2+local[0]/4]));
            }
            for col in 0..2 {
                let neighbour=super::avc_mbaff::neighbour_location(address,[(col*4) as isize,-1],self.width,self.cells.len()/self.width,true,[2,1],&mut pair_field)?;
                out.top[component][col]=neighbour.and_then(|(owner,local)|self.cells[owner].filter(|c|c.slice==slice).and_then(|_|wide[owner]).map(|counts|counts[component][local[1]/4*2+local[0]/4]));
            }
        }
        Ok(out)
    }
    pub(super) fn store_422(&mut self,address:usize,slice:u32,luma:[u8;16],chroma:[[u8;8];2])->Result<()> {
        if self.chroma422.is_none()||chroma.iter().flatten().any(|&v|v>16){return Err(invalid("invalid 422 coefficient counts"));}
        self.store(address,slice,luma,[[0;4];2])?;
        self.chroma422.as_mut().unwrap()[address]=Some(chroma);Ok(())
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
    /// Resolve external coefficient contexts in the current pair's sample
    /// layout. A boundary can touch more than one neighbouring macroblock.
    pub fn neighbours_mbaff(
        &self,
        address: usize,
        slice: u32,
        mut pair_field: impl FnMut(usize) -> Option<bool>,
    ) -> Result<CoefficientNeighbours> {
        let height = self.cells.len() / self.width;
        let field = pair_field(address / 2)
            .ok_or_else(|| invalid("MBAFF coefficient context lacks pair mode"))?;
        super::avc_mbaff::layout(address, self.width, height, true, field, [1, 1])?;
        let mut out = CoefficientNeighbours::default();
        for component in 0..3 {
            let n = if component == 0 { 4 } else { 2 };
            let sub = if component == 0 { [1, 1] } else { [2, 2] };
            for i in 0..n {
                for (offset, left) in [
                    ([-1, (i * 4) as isize], true),
                    ([(i * 4) as isize, -1], false),
                ] {
                    let location = super::avc_mbaff::neighbour_location(
                        address,
                        offset,
                        self.width,
                        height,
                        true,
                        sub,
                        &mut pair_field,
                    )?;
                    let value = location.and_then(|(owner, local)| {
                        self.cells[owner].filter(|c| c.slice == slice).map(|c| {
                            let index = local[1] / 4 * n + local[0] / 4;
                            if component == 0 {
                                c.luma[index]
                            } else {
                                c.chroma[component - 1][index]
                            }
                        })
                    });
                    match (component, left) {
                        (0, true) => out.luma_left[i] = value,
                        (0, false) => out.luma_top[i] = value,
                        (_, true) => out.chroma_left[component - 1][i] = value,
                        (_, false) => out.chroma_top[component - 1][i] = value,
                    }
                }
            }
        }
        Ok(out)
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
        if let Some(wide)=&mut self.chroma422 {wide[address]=Some([[0;8];2]);}
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
        self.read_inter_field(bits, address, slice, syntax, false)
    }
    pub fn read_inter_field(
        &mut self,
        bits: &mut BitReader<'_>,
        address: usize,
        slice: u32,
        syntax: &InterSyntax,
        field: bool,
    ) -> Result<(InterHeader, InterCoefficients)> {
        if syntax.chroma_array_type > 2 {
            return Err(invalid("inter coefficient field requires 4:2:0"));
        }
        let neighbours = self.neighbours(address, slice)?;
        if self.cells[address].is_some() {
            return Err(invalid("AVC coefficient macroblock already decoded"));
        }
        let mut input = bits.clone();
        let header = super::avc_inter::read_inter_header_field(&mut input, syntax, field)?;
        let coefficients = if syntax.chroma_array_type==2 {
            super::avc_inter_coefficients::read_inter_coefficients_422_field(&mut input,header.residual.pattern,header.residual.transform8,neighbours,self.neighbours_422(address,slice)?,field)?
        }else{super::avc_inter_coefficients::read_inter_coefficients_field(
            &mut input,
            header.residual.pattern,
            header.residual.transform8,
            neighbours,
            field,
        )?};
        if let Some(wide)=&coefficients.chroma422 {self.store_422(address,slice,coefficients.luma_counts,wide.counts)?;}
        else{self.store(
            address,
            slice,
            coefficients.luma_counts,
            coefficients.chroma_counts,
        )?;}
        *bits = input;
        Ok((header, coefficients))
    }
    /// The caller consumes mb_field_decoding_flag and supplies expanded active
    /// reference counts. Cursor and coefficient storage commit only on success.
    pub fn read_inter_mbaff(
        &mut self,
        bits: &mut BitReader<'_>,
        address: usize,
        slice: u32,
        syntax: &InterSyntax,
        mut pair_field: impl FnMut(usize) -> Option<bool>,
    ) -> Result<(InterHeader, InterCoefficients)> {
        if !matches!(syntax.chroma_array_type,0..=2) {
            return Err(invalid("MBAFF coefficient field requires mono/420/422"));
        }
        let neighbours = self.neighbours_mbaff(address, slice, &mut pair_field)?;
        let field = pair_field(address / 2)
            .ok_or_else(|| invalid("MBAFF coefficient context lacks pair mode"))?;
        if self.cells[address].is_some() {
            return Err(invalid("AVC coefficient macroblock already decoded"));
        }
        let mut input = bits.clone();
        let header = super::avc_inter::read_inter_header_field(&mut input, syntax, field)?;
        let coefficients = if syntax.chroma_array_type==2 {
            super::avc_inter_coefficients::read_inter_coefficients_422_field(
                &mut input,header.residual.pattern,header.residual.transform8,neighbours,
                self.neighbours_422_mbaff(address,slice,&mut pair_field)?,field)?
        } else {super::avc_inter_coefficients::read_inter_coefficients_field(
            &mut input,
            header.residual.pattern,
            header.residual.transform8,
            neighbours,
            field,
        )?};
        if let Some(wide)=&coefficients.chroma422 {self.store_422(address,slice,coefficients.luma_counts,wide.counts)?;}
        else{self.store(
            address,
            slice,
            coefficients.luma_counts,
            coefficients.chroma_counts,
        )?;}
        *bits = input;
        Ok((header, coefficients))
    }
}
#[cfg(test)]
mod tests {
    use super::super::avc_slice::SliceType;
    use super::*;
    #[test]
    fn rectangular_mbaff_counts_select_physical_frame_and_field_owners() {
        for current_field in [false,true] {for foreign_top in [false,true] {
            let mut counts=CoefficientField::new_422(2,4,8192).unwrap();
            let values=|address:usize| std::array::from_fn(|component|std::array::from_fn(|cell|((address+component+cell)%17)as u8));
            for address in 0..6 {counts.store_422(address,if address==3 && foreign_top {9}else{0},[0;16],values(address)).unwrap();}
            let modes=[false,!current_field,!current_field,current_field];
            let neighbours=counts.neighbours_422_mbaff(6,0,|pair|Some(modes[pair])).unwrap();
            for component in 0..2 {
                let expected=if current_field {
                    [Some(values(4)[component][1]),Some(values(4)[component][5]),Some(values(5)[component][1]),Some(values(5)[component][5])]
                }else{
                    [Some(values(4)[component][1]),Some(values(4)[component][1]),Some(values(4)[component][3]),Some(values(4)[component][3])]
                };
                assert_eq!(neighbours.left[component],expected);
                assert_eq!(neighbours.top[component],if foreign_top {[None;2]}else{[Some(values(3)[component][6]),Some(values(3)[component][7])]});
            }
        }}
    }
    #[test]
    fn rectangular_422_contexts_use_all_four_left_rows_and_bottom_top_row() {
        for foreign_top in [false,true] {
            let mut field=CoefficientField::new_422(2,2,4096).unwrap();
            field.store_422(1,if foreign_top {7}else{0},[0;16],[[8,9,10,11,12,13,14,15],[15,14,13,12,11,10,9,8]]).unwrap();
            field.store_422(2,0,[0;16],[[1,0,4,3,7,6,8,15],[0,1,3,4,6,7,15,8]]).unwrap();
            let neighbours=field.neighbours_422(3,0).unwrap();
            assert_eq!(neighbours.left,[[Some(0),Some(3),Some(6),Some(15)],[Some(1),Some(4),Some(7),Some(8)]]);
            assert_eq!(neighbours.top,if foreign_top {[[None;2];2]}else{[[Some(14),Some(15)],[Some(9),Some(8)]]});
            assert!(field.store_422(2,0,[0;16],[[0;8];2]).is_err());
        }
    }
    #[test]
    fn mixed_pair_context_reads_each_actual_boundary_owner() {
        let mut field = CoefficientField::new(2, 2, 8192).unwrap();
        field
            .store(0, 7, std::array::from_fn(|i| i as u8), [[1, 2, 3, 4]; 2])
            .unwrap();
        field
            .store(
                1,
                7,
                std::array::from_fn(|i| 16 - i as u8),
                [[5, 6, 7, 8]; 2],
            )
            .unwrap();
        let n = field.neighbours_mbaff(2, 7, |p| Some(p == 1)).unwrap();
        assert_eq!(n.luma_left, [Some(3), Some(11), Some(13), Some(5)]);
        assert_eq!(n.luma_top, [None; 4]);
        assert_eq!(n.chroma_left, [[Some(2), Some(6)]; 2]);
        assert_eq!(
            field
                .neighbours_mbaff(2, 8, |p| Some(p == 1))
                .unwrap()
                .luma_left,
            [None; 4]
        );
        assert!(field.neighbours_mbaff(2, 7, |_| None).is_err());
        assert!(field.neighbours_mbaff(4, 7, |_| Some(true)).is_err());
    }
    #[test]
    fn mbaff_inter_cursor_and_counts_commit_together() {
        let mut field = CoefficientField::new(1, 2, 8192).unwrap();
        let syntax = InterSyntax {
            slice: SliceType::P,
            active_references: [1, 0],
            previous_qp: 26,
            bit_depth: 8,
            chroma_array_type: 1,
            transform8_enabled: false,
            direct8_inference: true,
        };
        let mut bad = BitReader::new(&[0xe0]);
        assert!(
            field
                .read_inter_mbaff(&mut bad, 0, 7, &syntax, |_| Some(true))
                .is_err()
        );
        assert_eq!(bad.position(), 0);
        assert!(field.cells.iter().all(Option::is_none));
        let mut bits = BitReader::new(&[0xf0]);
        let (_, c) = field
            .read_inter_mbaff(&mut bits, 0, 7, &syntax, |_| Some(true))
            .unwrap();
        assert_eq!(bits.position(), 4);
        assert_eq!(c.luma_counts, [0; 16]);
        let before = bits.position();
        assert!(
            field
                .read_inter_mbaff(&mut bits, 0, 7, &syntax, |_| Some(true))
                .is_err()
        );
        assert_eq!(bits.position(), before);
    }
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
