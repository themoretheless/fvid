//! Immutable coded-CU geometry for Annex I current-AU texture prediction.
use crate::codec::hevc_inter_syntax::Partition;
use crate::{Result, invalid};
use std::sync::Arc;
#[derive(Clone)]
pub(crate) struct Map {
    pub(super) dimensions: [u32; 2],
    entries: Arc<[u8]>,
    motion: Option<Arc<[super::Motion]>>,
    iv: Option<Arc<[Option<crate::codec::hevc_motion::disparity::derivation::TextureDisparity>]>>,
}
impl Map {
    pub(super) fn new(dimensions: [u32; 2], entries: Vec<u8>) -> Result<Self> {
        let [w, h] = dimensions;
        if w == 0 || h == 0 || w % 8 != 0 || h % 8 != 0 {
            return Err(invalid("invalid HEVC coding-map dimensions"));
        }
        let width = w as usize / 8;
        let height = h as usize / 8;
        if width.checked_mul(height) != Some(entries.len()) {
            return Err(invalid("invalid HEVC coding-map length"));
        }
        for y in 0..height {
            for x in 0..width {
                let value = entries[y * width + x];
                if value > 63 {
                    return Err(invalid("invalid HEVC coding-map entry"));
                }
                let span = 1usize << ((value & 31) >> 3);
                let ox = x & !(span - 1);
                let oy = y & !(span - 1);
                if ox + span > width || oy + span > height || entries[oy * width + ox] != value {
                    return Err(invalid("inconsistent HEVC coding-map coding unit"));
                }
                if x == ox && y == oy {
                    for row in oy..oy + span {
                        if entries[row * width + ox..row * width + ox + span]
                            .iter()
                            .any(|&v| v != value)
                        {
                            return Err(invalid("inconsistent HEVC coding-map coding unit"));
                        }
                    }
                }
            }
        }
        Ok(Self {
            dimensions,
            entries: entries.into(),
            motion: None,
            iv: None,
        })
    }
    pub(super) fn with_motion(mut self, motion: Vec<super::Motion>) -> Result<Self> {
        if motion.len() != self.entries.len() {
            return Err(invalid("HEVC exact texture-motion map length disagrees"));
        }
        self.motion = Some(motion.into());
        Ok(self)
    }
    pub(super) fn motion_at(&self, point: [u32; 2]) -> Result<super::Motion> {
        self.at(point)?;
        let motion = self.motion.as_ref()
            .ok_or_else(|| invalid("HEVC picture has no exact texture-motion map"))?;
        Ok(motion[(point[1] as usize / 8) * (self.dimensions[0] as usize / 8) + point[0] as usize / 8])
    }
    pub(super) fn skipped_at(&self,point:[u32;2])->Result<bool> {
        self.at(point)?;
        Ok(self.entries[(point[1]as usize/8)*(self.dimensions[0]as usize/8)+point[0]as usize/8]&32!=0)
    }
    pub(super) fn with_iv(mut self,iv:Option<Vec<Option<crate::codec::hevc_motion::disparity::derivation::TextureDisparity>>>)->Result<Self> {
        if let Some(iv)=iv {
            if iv.len()!=self.entries.len() {return Err(invalid("HEVC saved IV map length disagrees"));}
            for (index,state)in iv.iter().enumerate() {
                if let Some(state)=state {
                    if state.vector.reference_view>=64 || self.motion.as_ref().is_none_or(|m|m[index]==[None;2]) {
                        return Err(invalid("invalid HEVC saved IV state"));
                    }
                }
            }
            if iv.iter().any(Option::is_some) {self.iv=Some(iv.into());}
        }
        Ok(self)
    }
    pub(super) fn iv_at(&self,point:[u32;2])->Result<Option<crate::codec::hevc_motion::disparity::derivation::TextureDisparity>> {
        self.at(point)?;
        Ok(self.iv.as_ref().and_then(|v|v[(point[1]as usize/8)*(self.dimensions[0]as usize/8)+point[0]as usize/8]))
    }
    pub(super) fn at(&self, point: [u32; 2]) -> Result<(u8, Partition)> {
        if point[0] >= self.dimensions[0] || point[1] >= self.dimensions[1] {
            return Err(invalid("HEVC coding-map position outside picture"));
        }
        let value = self.entries
            [(point[1] as usize / 8) * (self.dimensions[0] as usize / 8) + point[0] as usize / 8];
        let part = match value & 7 {
            0 => Partition::Full,
            1 => Partition::Horizontal,
            2 => Partition::Vertical,
            3 => Partition::Quarters,
            4 => Partition::Top,
            5 => Partition::Bottom,
            6 => Partition::Left,
            _ => Partition::Right,
        };
        Ok((3 + ((value & 31) >> 3), part))
    }
    pub(crate) fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.entries, &other.entries)
            && match (&self.motion, &other.motion) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
            && match (&self.iv,&other.iv) {
                (None,None)=>true,(Some(a),Some(b))=>Arc::ptr_eq(a,b),_=>false,
            }
    }
    pub(crate) fn heap_bytes(&self) -> Option<usize> {
        let align = std::mem::align_of::<usize>();
        let entries = self.entries
            .len()
            .checked_add(2 * std::mem::size_of::<usize>())?
            .checked_add(align - 1)
            .map(|n| n & !(align - 1))?;
        let motion = match &self.motion {
            None => 0,
            Some(motion) => motion.len().checked_mul(std::mem::size_of::<super::Motion>())?
                .checked_add(2 * std::mem::size_of::<usize>())?
                .checked_add(align - 1)? & !(align - 1),
        };
        let iv=match &self.iv {
            None=>0,Some(iv)=>iv.len().checked_mul(std::mem::size_of::<Option<crate::codec::hevc_motion::disparity::derivation::TextureDisparity>>())?
                .checked_add(2*std::mem::size_of::<usize>())?.checked_add(align-1)? & !(align-1),
        };
        entries.checked_add(motion)?.checked_add(iv)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iv_disparity_state_is_optional_shared_and_counted() {
        use crate::codec::hevc_motion::{Vector,disparity::derivation::{TextureDisparity,Vector as Disp}};
        let plain=Map::new([16,16],vec![8;4]).unwrap()
            .with_motion(vec![[Some(Vector{reference:0,mv:[0,4]}),None];4]).unwrap();
        let state=TextureDisparity{vector:Disp{reference_view:2,mv:[-37,4]},refined_mv:[91,0]};
        assert!(plain.clone().with_iv(Some(vec![None;3])).is_err());
        let empty=plain.clone().with_iv(Some(vec![None;4])).unwrap();
        assert_eq!(plain.heap_bytes(),empty.heap_bytes());
        assert!(plain.shares_storage(&empty));
        let map=plain.clone().with_iv(Some(vec![Some(state),None,None,Some(state)])).unwrap();
        assert!(map.shares_storage(&map.clone()));
        assert!(!map.shares_storage(&plain));
        assert!(map.heap_bytes().unwrap()>=plain.heap_bytes().unwrap()+4*std::mem::size_of::<Option<TextureDisparity>>());
        for y in 0..16 {for x in 0..16 {
            assert_eq!(map.iv_at([x,y]).unwrap(),if (x<8)==(y<8){Some(state)}else{None});
            assert_eq!(map.at([x,y]).unwrap(),(4,Partition::Full));
        }}
        assert!(map.iv_at([16,0]).is_err());
        assert!(Map::new([16,16],vec![8;4]).unwrap().with_iv(Some(vec![Some(state);4])).is_err());
    }
    #[test]
    fn skip_flag_shares_geometry_storage_without_changing_cu_size_or_partition() {
        for log in 3..=6 {
            for part in 0..8 {
                let value=((log-3)<<3)|part;
                let ordinary=Map::new([64,64],vec![value;64]).unwrap();
                let skipped=Map::new([64,64],vec![value|32;64]).unwrap();
                assert_eq!(ordinary.heap_bytes(),skipped.heap_bytes());
                for point in [[0,0],[7,7],[31,40],[63,63]] {
                    assert_eq!(ordinary.at(point).unwrap(),skipped.at(point).unwrap());
                    assert!(!ordinary.skipped_at(point).unwrap());
                    assert!(skipped.skipped_at(point).unwrap());
                }
                assert!(skipped.skipped_at([64,0]).is_err());
                assert!(skipped.shares_storage(&skipped.clone()));
            }
        }
    }
    #[test]
    fn exact_motion_is_retained_at_eight_sample_grid_and_shared_with_cu_metadata() {
        use crate::codec::hevc_motion::Vector;
        let empty = Map::new([16, 16], vec![8; 4]).unwrap();
        assert!(empty.motion_at([0, 0]).is_err());
        assert!(empty.clone().with_motion(vec![[None; 2]; 3]).is_err());
        let entries: Vec<_> = (0..4).map(|i| [Some(Vector { reference: i, mv: [i64::from(i), -1] }), None]).collect();
        let map = empty.clone().with_motion(entries.clone()).unwrap();
        assert!(!map.shares_storage(&empty));
        assert!(map.shares_storage(&map.clone()));
        assert!(map.heap_bytes().unwrap() >= empty.heap_bytes().unwrap() + 4 * std::mem::size_of::<super::super::Motion>());
        for y in 0..16 { for x in 0..16 {
            assert_eq!(map.motion_at([x, y]).unwrap(), entries[(y as usize / 8) * 2 + x as usize / 8]);
        }}
        assert!(map.motion_at([0, 16]).is_err());
    }
    #[test]
    fn every_partition_roundtrips_and_retained_heap_is_aligned() {
        for part in 0..8 {
            let map = Map::new([32, 32], vec![16 | part; 16]).unwrap();
            let (log, actual) = map.at([31, 31]).unwrap();
            assert_eq!((log, actual as u8), (5, part));
            assert_eq!(map.heap_bytes().unwrap() % std::mem::align_of::<usize>(), 0);
            assert!(map.shares_storage(&map.clone()));
        }
    }
    #[test]
    fn absolute_cu_sizes_partitions_consistency_and_shared_storage() {
        let value = ((5 - 3) << 3) | 4;
        let map = Map::new([64, 32], vec![value; 32]).unwrap();
        assert_eq!(map.at([31, 31]).unwrap(), (5, Partition::Top));
        assert_eq!(map.at([63, 0]).unwrap(), (5, Partition::Top));
        assert!(map.at([64, 0]).is_err());
        let copy = map.clone();
        assert!(Arc::ptr_eq(&map.entries, &copy.entries));
        let mut bad = vec![value; 32];
        bad[1] = 0;
        assert!(Map::new([64, 32], bad).is_err());
        assert!(Map::new([24, 16], vec![16; 6]).is_err());
        assert!(Map::new([8, 8], vec![64]).is_err());
        assert!(Map::new([7, 8], vec![]).is_err());
    }
}
