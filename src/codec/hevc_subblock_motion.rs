//! Shared Annex I-150..I-163 partition/default inheritance, before source lookup.
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    rect: [u32; 4],
    size: [u32; 2],
}
impl Layout {
    pub fn new(rect: [u32; 4], dimensions: [u32; 2], log2_size: u8) -> Result<Self> {
        let [x, y, w, h] = rect;
        if !(3..=6).contains(&log2_size)
            || w == 0
            || h == 0
            || w > 64
            || h > 64
            || x.checked_add(w).is_none_or(|end| end > dimensions[0])
            || y.checked_add(h).is_none_or(|end| end > dimensions[1])
        {
            return Err(invalid("invalid HEVC inter-layer sub-block geometry"));
        }
        let min = 1u32 << log2_size;
        Ok(Self {
            rect,
            size: if w % min != 0 || h % min != 0 {
                [w, h]
            } else {
                [min, min]
            },
        })
    }
    pub fn size(self) -> [u32; 2] {
        self.size
    }
    pub fn count(self) -> usize {
        (self.rect[2] / self.size[0] * (self.rect[3] / self.size[1])) as usize
    }
    pub fn rectangle(self, index: usize) -> Result<[u32; 4]> {
        if index >= self.count() {
            return Err(invalid("HEVC inter-layer sub-block index outside layout"));
        }
        let columns = self.rect[2] / self.size[0];
        Ok([
            self.rect[0] + index as u32 % columns * self.size[0],
            self.rect[1] + index as u32 / columns * self.size[1],
            self.size[0],
            self.size[1],
        ])
    }
    pub fn default_rectangle(self) -> [u32; 4] {
        [
            self.rect[0] + (self.rect[2] / self.size[0] / 2) * self.size[0],
            self.rect[1] + (self.rect[3] / self.size[1] / 2) * self.size[1],
            self.size[0],
            self.size[1],
        ]
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subblocks<T> {
    pub size: [u32; 2],
    pub default: T,
    pub motion: Vec<T>,
}
impl<T> Subblocks<T> {
    pub fn required_bytes(layout: Layout) -> Result<usize> {
        layout
            .count()
            .checked_mul(std::mem::size_of::<T>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>()))
            .ok_or_else(|| invalid("HEVC inter-layer sub-block storage overflow"))
    }
}
/// Only a nonempty default enables the candidate. A sub-block inherits the
/// entire default only when all of its prediction lists are inactive.
/// The rectangle passed to `derive` is unshifted; source-specific alignment,
/// disparity and clipping belong inside that callback, not this partitioner.
pub fn derive<T: Copy + PartialEq>(
    layout: Layout,
    inactive: T,
    budget: usize,
    mut derive: impl FnMut([u32; 4]) -> Result<T>,
) -> Result<Subblocks<T>> {
    if budget < std::mem::size_of::<Subblocks<T>>() {
        return Err(invalid(
            "HEVC inter-layer sub-block metadata exceeds budget",
        ));
    }
    let default = derive(layout.default_rectangle())?;
    let mut motion = Vec::new();
    if default != inactive {
        if Subblocks::<T>::required_bytes(layout)? > budget {
            return Err(invalid("HEVC inter-layer sub-block motion exceeds budget"));
        }
        motion
            .try_reserve_exact(layout.count())
            .map_err(|_| invalid("HEVC inter-layer sub-block allocation failed"))?;
        let retained = motion
            .capacity()
            .checked_mul(std::mem::size_of::<T>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Subblocks<T>>()))
            .ok_or_else(|| invalid("HEVC inter-layer sub-block storage overflow"))?;
        if retained > budget {
            return Err(invalid("HEVC inter-layer sub-block motion exceeds budget"));
        }
        for index in 0..layout.count() {
            let value = derive(layout.rectangle(index)?)?;
            motion.push(if value == inactive { default } else { value });
        }
    }
    Ok(Subblocks {
        size: layout.size,
        default,
        motion,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_default_stops_before_other_subblocks_and_partial_motion_is_retained() {
        let layout = Layout::new([0, 0, 16, 16], [16, 16], 3).unwrap();
        let mut calls = 0;
        let result = derive(
            layout,
            [None::<i32>; 2],
            std::mem::size_of::<Subblocks<[Option<i32>; 2]>>(),
            |rect| {
                calls += 1;
                assert_eq!(rect, [8, 8, 8, 8]);
                Ok([None; 2])
            },
        )
        .unwrap();
        assert!(result.motion.is_empty());
        assert_eq!(calls, 1);
        let result = derive(layout, [None; 2], 4096, |rect| {
            Ok(if rect[0] == 0 {
                if rect[1] == 0 {
                    [None, None]
                } else {
                    [None, Some(7)]
                }
            } else {
                [Some(3), Some(4)]
            })
        })
        .unwrap();
        assert_eq!(
            result.motion,
            [
                [Some(3), Some(4)],
                [Some(3), Some(4)],
                [None, Some(7)],
                [Some(3), Some(4)]
            ]
        );
    }
    #[test]
    fn whole_block_fallback_bounds_and_exact_budget() {
        let layout = Layout::new([8, 8, 12, 16], [32, 32], 3).unwrap();
        assert_eq!(layout.size(), [12, 16]);
        assert_eq!(layout.count(), 1);
        assert_eq!(layout.default_rectangle(), [8, 8, 12, 16]);
        let bytes = Subblocks::<u32>::required_bytes(layout).unwrap();
        assert!(derive(layout, 0, bytes - 1, |_| Ok(1)).is_err());
        assert_eq!(derive(layout, 0, bytes, |_| Ok(1)).unwrap().motion, [1]);
        assert!(layout.rectangle(1).is_err());
        assert!(Layout::new([0, 0, 65, 8], [128, 128], 3).is_err());
        assert!(Layout::new([u32::MAX, 0, 8, 8], [32, 32], 3).is_err());
    }
}
