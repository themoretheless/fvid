//! VP9 v0.7 sections 6.3.18 and 9.3.2: reference ordering and entropy contexts.
type Ref = [u8; 2];
fn single(r: Ref) -> bool {
    r[1] == 0
}
fn intra(r: Ref) -> bool {
    r[0] == 0
}
fn has(r: Ref, v: u8) -> usize {
    usize::from(r.contains(&v))
}
pub(super) fn layout(bias: [bool; 3]) -> (u8, [u8; 2], usize) {
    let (fixed, var) = if bias[0] == bias[1] {
        (3, [1, 2])
    } else if bias[0] == bias[2] {
        (2, [1, 3])
    } else {
        (1, [2, 3])
    };
    (fixed, var, usize::from(bias[usize::from(fixed - 1)]))
}
pub(super) fn mode(a: Option<Ref>, l: Option<Ref>, fixed: u8) -> usize {
    match (a, l) {
        (Some(a), Some(l)) if single(a) && single(l) => {
            usize::from((a[0] == fixed) ^ (l[0] == fixed))
        }
        (Some(a), Some(_)) if single(a) => 2 + usize::from(a[0] == fixed || intra(a)),
        (Some(_), Some(l)) if single(l) => 2 + usize::from(l[0] == fixed || intra(l)),
        (Some(_), Some(_)) => 4,
        (Some(r), None) | (None, Some(r)) => {
            if single(r) {
                usize::from(r[0] == fixed)
            } else {
                3
            }
        }
        _ => 1,
    }
}
pub(super) fn compound(
    a: Option<Ref>,
    l: Option<Ref>,
    fixed: u8,
    var: [u8; 2],
    fix_idx: usize,
) -> usize {
    let vr = |r: Ref| if single(r) { r[0] } else { r[1 - fix_idx] };
    match (a, l) {
        (Some(a), Some(l)) if intra(a) && intra(l) => 2,
        (Some(a), Some(l)) if intra(a) || intra(l) => {
            1 + 2 * usize::from(vr(if intra(a) { l } else { a }) != var[1])
        }
        (Some(a), Some(l)) => {
            let (va, vl) = (vr(a), vr(l));
            if va == vl && va == var[1] {
                0
            } else if single(a) && single(l) {
                if (va == fixed && vl == var[0]) || (vl == fixed && va == var[0]) {
                    4
                } else if va == vl {
                    3
                } else {
                    1
                }
            } else if single(a) || single(l) {
                let (vc, vs) = if single(a) { (vl, va) } else { (va, vl) };
                if vc == var[1] && vs != var[1] {
                    1
                } else if vs == var[1] && vc != var[1] {
                    2
                } else {
                    4
                }
            } else if va == vl {
                4
            } else {
                2
            }
        }
        (Some(r), None) | (None, Some(r)) => {
            if intra(r) {
                2
            } else {
                (if single(r) { 3 } else { 4 }) * usize::from(vr(r) != var[1])
            }
        }
        _ => 2,
    }
}
pub(super) fn single_first(a: Option<Ref>, l: Option<Ref>) -> usize {
    let one = |r: Ref| {
        if intra(r) {
            2
        } else if single(r) {
            4 * has(r, 1)
        } else {
            1 + has(r, 1)
        }
    };
    match (a, l) {
        (Some(a), Some(l)) if intra(a) && intra(l) => 2,
        (Some(a), Some(l)) if intra(a) || intra(l) => one(if intra(a) { l } else { a }),
        (Some(a), Some(l)) if single(a) && single(l) => 2 * has(a, 1) + 2 * has(l, 1),
        (Some(a), Some(l)) if !single(a) && !single(l) => {
            1 + usize::from(has(a, 1) + has(l, 1) > 0)
        }
        (Some(a), Some(l)) => {
            let (s, c) = if single(a) { (a, l) } else { (l, a) };
            if s[0] == 1 { 3 + has(c, 1) } else { has(c, 1) }
        }
        (Some(r), None) | (None, Some(r)) => one(r),
        _ => 2,
    }
}
pub(super) fn single_second(a: Option<Ref>, l: Option<Ref>) -> usize {
    let beside_intra = |r: Ref| {
        if single(r) {
            if r[0] == 1 { 3 } else { 4 * has(r, 2) }
        } else {
            1 + 2 * has(r, 2)
        }
    };
    match (a, l) {
        (Some(a), Some(l)) if intra(a) && intra(l) => 2,
        (Some(a), Some(l)) if intra(a) || intra(l) => beside_intra(if intra(a) { l } else { a }),
        (Some(a), Some(l)) if single(a) && single(l) => {
            if a[0] == 1 && l[0] == 1 {
                3
            } else if a[0] == 1 {
                4 * has(l, 2)
            } else if l[0] == 1 {
                4 * has(a, 2)
            } else {
                2 * has(a, 2) + 2 * has(l, 2)
            }
        }
        (Some(a), Some(l)) if !single(a) && !single(l) => {
            if a == l {
                3 * has(a, 2)
            } else {
                2
            }
        }
        (Some(a), Some(l)) => {
            let (s, c) = if single(a) { (a, l) } else { (l, a) };
            match s[0] {
                2 => 3 + has(c, 2),
                3 => has(c, 2),
                _ => 1 + 2 * has(c, 2),
            }
        }
        (Some(r), None) | (None, Some(r)) => {
            if intra(r) || (single(r) && r[0] == 1) {
                2
            } else if single(r) {
                4 * has(r, 2)
            } else {
                3 * has(r, 2)
            }
        }
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_reference_order_covers_every_nonuniform_sign_bias() {
        for (bias, expected) in [
            ([false, false, true], (3, [1, 2], 1)),
            ([true, true, false], (3, [1, 2], 0)),
            ([false, true, false], (2, [1, 3], 1)),
            ([true, false, true], (2, [1, 3], 0)),
            ([true, false, false], (1, [2, 3], 1)),
            ([false, true, true], (1, [2, 3], 0)),
        ] {
            assert_eq!(layout(bias), expected);
        }
    }
    #[test]
    fn reference_contexts_cover_absent_intra_single_and_compound_neighbours() {
        let n = None;
        let i = Some([0, 0]);
        let last = Some([1, 0]);
        let gold = Some([2, 0]);
        let alt = Some([3, 0]);
        let la = Some([1, 3]);
        let ga = Some([2, 3]);
        // Explicit section 9.3.2 results; each tuple gives mode/comp/p1/p2.
        for (a, l, expected) in [
            (n, n, [1, 2, 2, 2]),
            (i, i, [0, 2, 2, 2]),
            (last, n, [0, 3, 4, 2]),
            (gold, n, [0, 0, 0, 4]),
            (alt, n, [1, 3, 0, 0]),
            (la, n, [3, 4, 2, 0]),
            (ga, n, [3, 0, 1, 3]),
            (la, ga, [4, 2, 2, 2]),
            (ga, ga, [4, 0, 1, 3]),
            (la, la, [4, 4, 2, 0]),
            (last, alt, [1, 4, 2, 0]),
            (gold, i, [0, 1, 0, 4]),
            (last, i, [0, 3, 4, 3]),
            (la, i, [3, 3, 2, 1]),
            (ga, i, [3, 1, 1, 3]),
            (la, last, [2, 4, 4, 1]),
            (ga, last, [2, 1, 3, 3]),
            (la, gold, [2, 2, 1, 3]),
            (ga, gold, [2, 0, 0, 4]),
            (la, alt, [3, 4, 1, 0]),
            (ga, alt, [3, 1, 0, 1]),
        ] {
            assert_eq!(
                [
                    mode(a, l, 3),
                    compound(a, l, 3, [1, 2], 1),
                    single_first(a, l),
                    single_second(a, l)
                ],
                expected,
                "{a:?} {l:?}"
            );
            assert_eq!(
                [
                    mode(l, a, 3),
                    compound(l, a, 3, [1, 2], 1),
                    single_first(l, a),
                    single_second(l, a)
                ],
                expected,
                "reverse {a:?} {l:?}"
            );
        }
    }
}
