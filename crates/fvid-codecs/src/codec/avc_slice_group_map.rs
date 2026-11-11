//! Owned FMO map-unit address derivation, H.264 8.2.2.1–7.
use super::avc::SliceGroups;
use crate::{Result, invalid};
/// Derive map units before allocating picture state. `budget` bounds map bytes.
pub fn map_units(
    groups: &SliceGroups,
    width: usize,
    height: usize,
    cycle: Option<u32>,
    budget: usize,
) -> Result<Vec<u8>> {
    let count = width
        .checked_mul(height)
        .ok_or_else(|| invalid("FMO map size overflow"))?;
    if width == 0 || height == 0 || count > budget || count > 1 << 20 {
        return Err(invalid("FMO map exceeds dimensions or byte budget"));
    }
    let mut out = vec![0; count];
    match groups {
        SliceGroups::Single => {}
        SliceGroups::Interleaved(runs) => {
            if !(2..=8).contains(&runs.len()) || runs.contains(&0) {
                return Err(invalid("invalid FMO runs"));
            }
            let mut at = 0;
            while at < count {
                for (group, run) in runs.iter().enumerate() {
                    let end = at.saturating_add(*run as usize).min(count);
                    out[at..end].fill(group as u8);
                    at = end;
                }
            }
        }
        SliceGroups::Dispersed(groups) => {
            if !(2..=8).contains(groups) {
                return Err(invalid("invalid dispersed FMO group count"));
            }
            for (i, value) in out.iter_mut().enumerate() {
                *value =
                    ((i % width + (i / width * (*groups as usize)) / 2) % (*groups as usize)) as u8;
            }
        }
        SliceGroups::Foreground(rectangles) => {
            if !(1..=7).contains(&rectangles.len()) {
                return Err(invalid("invalid FMO rectangle count"));
            }
            out.fill(rectangles.len() as u8);
            for (group, &(top, bottom)) in rectangles.iter().enumerate().rev() {
                let (top, bottom) = (top as usize, bottom as usize);
                if top >= count
                    || bottom >= count
                    || top / width > bottom / width
                    || top % width > bottom % width
                {
                    return Err(invalid("invalid FMO rectangle"));
                }
                for y in top / width..=bottom / width {
                    out[y * width + top % width..=y * width + bottom % width].fill(group as u8);
                }
            }
        }
        SliceGroups::Explicit { groups, map } => {
            if !(2..=8).contains(groups)
                || map.len() != count
                || map.iter().any(|g| u32::from(*g) >= *groups)
            {
                return Err(invalid("invalid explicit FMO map"));
            }
            out.copy_from_slice(map);
        }
        SliceGroups::Changing {
            map_type,
            direction,
            rate,
        } => {
            let cycle = cycle.ok_or_else(|| invalid("missing FMO change cycle"))? as usize;
            if *rate == 0 || !(3..=5).contains(map_type) || cycle > count.div_ceil(*rate as usize) {
                return Err(invalid("invalid FMO change cycle or rate"));
            }
            let zero = cycle
                .checked_mul(*rate as usize)
                .ok_or_else(|| invalid("FMO change size overflow"))?
                .min(count);
            let d = usize::from(*direction);
            let upper = if *direction { count - zero } else { zero };
            if *map_type == 4 {
                for (i, g) in out.iter_mut().enumerate() {
                    *g = if i < upper { d as u8 } else { (1 - d) as u8 };
                }
            } else if *map_type == 5 {
                for x in 0..width {
                    for y in 0..height {
                        out[y * width + x] = if x * height + y < upper {
                            d as u8
                        } else {
                            (1 - d) as u8
                        };
                    }
                }
            } else {
                out.fill(1);
                let (mut x, mut y) = (((width - d) / 2) as isize, ((height - d) / 2) as isize);
                let (mut left, mut right, mut top, mut bottom) = (x, x, y, y);
                let (mut dx, mut dy) = (d as isize - 1, d as isize);
                let mut assigned = 0;
                while assigned < zero {
                    let slot = &mut out[y as usize * width + x as usize];
                    if *slot == 1 {
                        *slot = 0;
                        assigned += 1;
                    }
                    if dx == -1 && x == left {
                        left = (left - 1).max(0);
                        x = left;
                        dx = 0;
                        dy = 2 * d as isize - 1;
                    } else if dx == 1 && x == right {
                        right = (right + 1).min(width as isize - 1);
                        x = right;
                        dx = 0;
                        dy = 1 - 2 * d as isize;
                    } else if dy == -1 && y == top {
                        top = (top - 1).max(0);
                        y = top;
                        dx = 1 - 2 * d as isize;
                        dy = 0;
                    } else if dy == 1 && y == bottom {
                        bottom = (bottom + 1).min(height as isize - 1);
                        y = bottom;
                        dx = 2 * d as isize - 1;
                        dy = 0;
                    } else {
                        x += dx;
                        y += dy;
                    }
                }
            }
        }
    }
    Ok(out)
}
/// Convert map units to macroblock addresses (H.264 8.2.2.8).
pub fn macroblocks(
    map: &[u8],
    width: usize,
    frame_only: bool,
    field_picture: bool,
    mbaff_frame: bool,
    budget: usize,
) -> Result<Vec<u8>> {
    if width == 0
        || map.is_empty()
        || map.len() % width != 0
        || (mbaff_frame && (frame_only || field_picture))
        || (frame_only && field_picture)
    {
        return Err(invalid("invalid FMO macroblock map geometry"));
    }
    let count = map
        .len()
        .checked_mul(if frame_only || field_picture { 1 } else { 2 })
        .ok_or_else(|| invalid("FMO macroblock map overflow"))?;
    if count > budget {
        return Err(invalid("FMO macroblock map exceeds byte budget"));
    }
    Ok((0..count)
        .map(|i| {
            map[if frame_only || field_picture {
                i
            } else if mbaff_frame {
                i / 2
            } else {
                (i / (2 * width)) * width + i % width
            }]
        })
        .collect())
}
/// NextMbAddress skips map units belonging to other slice groups.
pub fn next_address(map: &[u8], address: usize) -> Result<usize> {
    let group = *map
        .get(address)
        .ok_or_else(|| invalid("FMO address outside map"))?;
    Ok((address + 1..map.len())
        .find(|i| map[*i] == group)
        .unwrap_or(map.len()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_maps_and_next_addresses() {
        assert_eq!(
            map_units(&SliceGroups::Interleaved(vec![1, 2]), 4, 2, None, 8).unwrap(),
            [0, 1, 1, 0, 1, 1, 0, 1]
        );
        let map = map_units(&SliceGroups::Dispersed(2), 4, 2, None, 8).unwrap();
        assert_eq!(map, [0, 1, 0, 1, 1, 0, 1, 0]);
        assert_eq!(next_address(&map, 0).unwrap(), 2);
        assert_eq!(next_address(&map, 7).unwrap(), 8);
        assert_eq!(
            map_units(
                &SliceGroups::Foreground(vec![(1, 6), (0, 7)]),
                4,
                2,
                None,
                8
            )
            .unwrap(),
            [1, 0, 0, 1, 1, 0, 0, 1]
        );
        assert_eq!(
            map_units(
                &SliceGroups::Explicit {
                    groups: 2,
                    map: map.clone()
                },
                4,
                2,
                None,
                8
            )
            .unwrap(),
            map
        );
    }
    #[test]
    fn changing_maps_cover_every_small_shape_cycle_and_direction() {
        for width in 1usize..=9 {
            for height in 1usize..=9 {
                for direction in [false, true] {
                    for rate in [1, 2, 5] {
                        for cycle in 0..=(width * height).div_ceil(rate) {
                            for map_type in 3..=5 {
                                let map = map_units(
                                    &SliceGroups::Changing {
                                        map_type,
                                        direction,
                                        rate: rate as u32,
                                    },
                                    width,
                                    height,
                                    Some(cycle as u32),
                                    width * height,
                                )
                                .unwrap();
                                assert_eq!(
                                    map.iter().filter(|g| **g == 0).count(),
                                    (cycle * rate).min(width * height)
                                );
                                assert!(map.iter().all(|g| *g <= 1));
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn changing_map_spatial_order_matches_fixed_examples() {
        for (map_type, direction, expected) in [
            (3, false, [0, 0, 0, 0, 0, 1, 1, 1, 1]),
            (3, true, [1, 1, 0, 1, 0, 0, 1, 0, 0]),
            (4, false, [0, 0, 0, 0, 0, 1, 1, 1, 1]),
            (5, false, [0, 0, 1, 0, 0, 1, 0, 1, 1]),
            (5, true, [1, 1, 0, 1, 0, 0, 1, 0, 0]),
        ] {
            assert_eq!(
                map_units(
                    &SliceGroups::Changing {
                        map_type,
                        direction,
                        rate: 1
                    },
                    3,
                    3,
                    Some(5),
                    9
                )
                .unwrap(),
                expected
            );
        }
    }
    #[test]
    fn map_unit_conversion_distinguishes_frame_field_and_mbaff() {
        let map = [0, 1, 2, 3];
        assert_eq!(macroblocks(&map, 2, true, false, false, 4).unwrap(), map);
        assert_eq!(macroblocks(&map, 2, false, true, false, 4).unwrap(), map);
        assert_eq!(
            macroblocks(&map, 2, false, false, true, 8).unwrap(),
            [0, 0, 1, 1, 2, 2, 3, 3]
        );
        assert_eq!(
            macroblocks(&map, 2, false, false, false, 8).unwrap(),
            [0, 1, 0, 1, 2, 3, 2, 3]
        );
        assert!(macroblocks(&map, 2, false, false, true, 7).is_err());
        assert!(macroblocks(&map, 3, false, false, true, 8).is_err());
        assert!(macroblocks(&map, 2, false, true, true, 8).is_err());
    }
    #[test]
    fn malformed_maps_refused() {
        assert!(map_units(&SliceGroups::Single, usize::MAX, 2, None, 8).is_err());
        assert!(map_units(&SliceGroups::Interleaved(vec![0, 1]), 4, 2, None, 8).is_err());
        assert!(map_units(&SliceGroups::Foreground(vec![(3, 4)]), 4, 2, None, 8).is_err());
        assert!(
            map_units(
                &SliceGroups::Explicit {
                    groups: 2,
                    map: vec![2; 8]
                },
                4,
                2,
                None,
                8
            )
            .is_err()
        );
        assert!(
            map_units(
                &SliceGroups::Changing {
                    map_type: 3,
                    direction: false,
                    rate: 1
                },
                4,
                2,
                Some(9),
                8
            )
            .is_err()
        );
        assert!(next_address(&[], 0).is_err());
    }
}
