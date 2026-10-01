//! Owned H.264 transform-bypass residual construction (8.5.15).
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    None,
    Horizontal,
    Vertical,
}
/// Accumulate intra residual DPCM in the complete prediction block, not per 4x4 tile.
pub fn residual<const N: usize>(
    levels: &[i32; N],
    width: usize,
    direction: Direction,
) -> Result<[i32; N]> {
    if width == 0 || N == 0 || !N.is_multiple_of(width) {
        return Err(invalid("invalid AVC bypass residual geometry"));
    }
    let mut result = *levels;
    for y in 0..N / width {
        for x in 0..width {
            let index = y * width + x;
            let previous = match direction {
                Direction::Horizontal if x > 0 => Some(index - 1),
                Direction::Vertical if y > 0 => Some(index - width),
                _ => None,
            };
            if let Some(previous) = previous {
                result[index] = result[index]
                    .checked_add(result[previous])
                    .ok_or_else(|| invalid("AVC bypass residual overflow"))?;
            }
        }
    }
    Ok(result)
}
pub fn luma_direction(mode: u8) -> Direction {
    match mode {
        0 => Direction::Vertical,
        1 => Direction::Horizontal,
        _ => Direction::None,
    }
}
/// Join raster 4x4 AC blocks and separately coded DC into a complete residual plane.
pub fn blocks4<const N: usize>(
    ac: &[[i32; 16]],
    dc: Option<&[i32]>,
    width: usize,
) -> Result<[i32; N]> {
    if width == 0
        || N == 0
        || !width.is_multiple_of(4)
        || !N.is_multiple_of(width)
        || !(N / width).is_multiple_of(4)
        || ac.len() != N / 16
        || dc.is_some_and(|d| d.len() != ac.len())
    {
        return Err(invalid("invalid AVC bypass coefficient geometry"));
    }
    let mut plane = [0; N];
    for (block, levels) in ac.iter().enumerate() {
        let (x, y) = (block % (width / 4) * 4, block / (width / 4) * 4);
        for i in 0..16 {
            plane[(y + i / 4) * width + x + i % 4] = if i == 0 {
                dc.map_or(levels[0], |d| d[block])
            } else {
                levels[i]
            };
        }
    }
    Ok(plane)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dpcm_direction_and_checked_overflow() {
        let levels = [1, 2, 3, 4, 5, 6];
        assert_eq!(
            residual(&levels, 3, Direction::Horizontal).unwrap(),
            [1, 3, 6, 4, 9, 15]
        );
        assert_eq!(
            residual(&levels, 3, Direction::Vertical).unwrap(),
            [1, 2, 3, 5, 7, 9]
        );
        assert_eq!(residual(&levels, 3, Direction::None).unwrap(), levels);
        assert!(residual(&[i32::MAX, 1], 2, Direction::Horizontal).is_err());
        assert!(residual(&levels, 0, Direction::None).is_err());
    }
    #[test]
    fn dc_tiles_are_joined_before_accumulation() {
        let levels = blocks4::<64>(&[[0; 16]; 4], Some(&[1, 2, 3, 4]), 8).unwrap();
        let out = residual(&levels, 8, Direction::Vertical).unwrap();
        assert_eq!([out[0], out[4], out[32], out[36]], [1, 2, 4, 6]);
    }
}
