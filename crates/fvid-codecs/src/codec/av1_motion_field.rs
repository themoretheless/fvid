//! Owned AV1 temporal motion-field projection (section 7.9; reference MV storage).
use super::*;
pub(super) const INVALID: [i32; 2] = [-32768; 2];
const DIV: [i32; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];
fn project(mv: [i32; 2], numerator: i32, denominator: i32) -> [i32; 2] {
    mv.map(|v| {
        let value = i64::from(v)
            * i64::from(numerator.clamp(-31, 31))
            * i64::from(DIV[denominator.clamp(1, 31) as usize]);
        (value.signum() * ((value.abs() + 8192) >> 14)).clamp(-16383, 16383) as i32
    })
}
fn position(v: usize, delta: i32, sign: i32, limit: usize, margin: i32) -> Option<usize> {
    let base = ((v >> 3) << 3) as i32;
    let next = v as i32 + sign * (delta / 64);
    (next >= 0 && next < limit as i32 && next >= base - margin && next < base + 8 + margin)
        .then_some(next as usize)
}
pub(super) fn hints(h: &Header, distances: [i32; 8], bits: u8) -> [u32; 8] {
    let mask = (1u32 << bits) - 1;
    distances.map(|d| h.order_hint.wrapping_add_signed(d) & mask)
}
pub(super) fn build(
    h: &Header,
    references: [Option<&Picture>; 8],
    distances: [i32; 8],
    grid: [usize; 2],
    bits: u8,
) -> Vec<[[i32; 2]; 8]> {
    if !h.reference_mvs {
        return Vec::new();
    }
    let [cols, rows] = grid;
    let width = cols / 2;
    let height = rows / 2;
    let mut field = vec![[INVALID; 8]; width * height];
    let current_hints = hints(h, distances, bits);
    let mut apply = |role: usize, sign: i32| {
        let Some(picture) = references[h.references[role - 1]] else {
            return false;
        };
        if picture.segment_grid != grid || matches!(picture.motion_frame_type, 0 | 2) {
            return false;
        }
        for y in 0..height {
            for x in 0..width {
                let saved = picture.saved_motion[y * width + x];
                if saved.reference == 0 {
                    continue;
                }
                let offset = -picture.motion_distances[usize::from(saved.reference)];
                let to_cur = distances[role];
                if offset <= 0 || offset > 31 || to_cur.abs() > 31 {
                    continue;
                }
                let mv = project(saved.mv, to_cur * sign, offset);
                let Some(px) = position(x, mv[1], sign, width, 8) else {
                    continue;
                };
                let Some(py) = position(y, mv[0], sign, height, 0) else {
                    continue;
                };
                for dst in 1..8 {
                    field[py * width + px][dst] = project(saved.mv, -distances[dst], offset);
                }
            }
        }
        true
    };
    if references[h.references[0]].is_some_and(|r| r.motion_hints[7] != current_hints[4]) {
        apply(1, -1);
    }
    let mut stamp = 1;
    for role in [5, 6, 7] {
        if distances[role] > 0 && (role != 7 || stamp >= 0) && apply(role, 1) {
            stamp -= 1;
        }
    }
    if stamp >= 0 {
        apply(2, -1);
    }
    field
}
pub(super) fn save(dec: &mut Decoder<'_>) {
    if !dec.s.reference_mvs {
        return;
    }
    let width = dec.cols / 2;
    for y in 0..dec.rows / 2 {
        for x in 0..width {
            let b = dec.blocks[(2 * y + 1) * dec.cols + 2 * x + 1];
            let mut saved = SavedMotion::default();
            for (reference, mv) in [(b.reference, b.mv), (b.reference2, b.mv2)] {
                if reference > 0
                    && dec.distances[reference] < 0
                    && mv.iter().all(|v| v.abs() <= 4095)
                {
                    saved = SavedMotion {
                        reference: reference as u8,
                        mv,
                    };
                }
            }
            dec.image.saved_motion[y * width + x] = saved;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_projection_and_clipping() {
        assert_eq!(project([64, -64], 1, 2), [32, -32]);
        assert_eq!(project([1, -1], 1, 2), [1, -1]);
        assert_eq!(project([4095, -4095], 31, 1), [16383, -16383]);
        assert_eq!(project([64, -64], -1, 2), [-32, 32]);
    }
    #[test]
    fn bounded_positions_use_truncated_negative_offsets() {
        assert_eq!(position(8, -63, 1, 32, 0), Some(8));
        assert_eq!(position(8, -64, 1, 32, 0), None);
        assert_eq!(position(8, -64, 1, 32, 8), Some(7));
        assert_eq!(position(0, 64, -1, 32, 8), None);
    }
}
