//! The 3D LUT engine measured against an outside implementation: `ffmpeg`'s
//! `lut3d` filter reads the same cube file and the same probe image and writes
//! the reference bytes committed beside them. `tests/fixtures/lut/README.md`
//! records the commands, the version and the hashes.
//!
//! What the comparison says is in the README; what these tests hold is the
//! agreement itself — one code at most on every channel of every pixel, for
//! each of the three interpolation modes, and the whole of that difference
//! explained by one rule: `ffmpeg` truncates the 8-bit result, Fvid rounds it.

use fvid::color::{Interpolation, Lut, Lut3d};

const CUBE: &str = include_str!("fixtures/lut/grade-17.cube");
const PRELUTED: &str = include_str!("fixtures/lut/preluted-17.cube");
const OVER: &str = include_str!("fixtures/lut/over-17.cube");
const THREE_DL: &str = include_str!("fixtures/lut/grade-17.3dl");
const DAT: &str = include_str!("fixtures/lut/grade-17.dat");
const PROBE: &[u8] = include_bytes!("fixtures/lut/probe.rgb");
const SIZE: usize = 17;

/// The reference `ffmpeg` wrote for one interpolation mode.
fn reference(mode: &str) -> Vec<u8> {
    match mode {
        "nearest" => include_bytes!("fixtures/lut/ffmpeg-nearest.rgb").to_vec(),
        "trilinear" => include_bytes!("fixtures/lut/ffmpeg-trilinear.rgb").to_vec(),
        "tetrahedral" => include_bytes!("fixtures/lut/ffmpeg-tetrahedral.rgb").to_vec(),
        other => unreachable!("no reference for {other}"),
    }
}

/// The reference `ffmpeg` wrote for the same grid with sections and vendor keys
/// written in front of it.
fn reference_preluted(mode: &str) -> Vec<u8> {
    match mode {
        "nearest" => include_bytes!("fixtures/lut/ffmpeg-preluted-nearest.rgb").to_vec(),
        "trilinear" => include_bytes!("fixtures/lut/ffmpeg-preluted-trilinear.rgb").to_vec(),
        "tetrahedral" => include_bytes!("fixtures/lut/ffmpeg-preluted-tetrahedral.rgb").to_vec(),
        other => unreachable!("no pre-LUT reference for {other}"),
    }
}

/// The reference `ffmpeg` wrote for the `.3dl` twin of the same look.
fn reference_3dl(mode: &str) -> Vec<u8> {
    match mode {
        "nearest" => include_bytes!("fixtures/lut/ffmpeg-3dl-nearest.rgb").to_vec(),
        "trilinear" => include_bytes!("fixtures/lut/ffmpeg-3dl-trilinear.rgb").to_vec(),
        "tetrahedral" => include_bytes!("fixtures/lut/ffmpeg-3dl-tetrahedral.rgb").to_vec(),
        other => unreachable!("no .3dl reference for {other}"),
    }
}

/// The reference `ffmpeg` wrote for the overshooting cube.
fn reference_over(mode: &str) -> Vec<u8> {
    match mode {
        "nearest" => include_bytes!("fixtures/lut/ffmpeg-over-nearest.rgb").to_vec(),
        "trilinear" => include_bytes!("fixtures/lut/ffmpeg-over-trilinear.rgb").to_vec(),
        "tetrahedral" => include_bytes!("fixtures/lut/ffmpeg-over-tetrahedral.rgb").to_vec(),
        other => unreachable!("no overshoot reference for {other}"),
    }
}

/// The `.3dl`'s rows, read in the order the file lists them, unscaled.
fn three_dl_rows() -> Vec<[f32; 3]> {
    THREE_DL
        .lines()
        .skip(1)
        .map(|line| {
            let mut parts = line.split_whitespace();
            let (Some(r), Some(g), Some(b)) = (parts.next(), parts.next(), parts.next()) else {
                panic!("a .3dl row has three channels: {line}");
            };
            [
                r.parse::<f32>().expect("red") / 4095.0,
                g.parse::<f32>().expect("green") / 4095.0,
                b.parse::<f32>().expect("blue") / 4095.0,
            ]
        })
        .collect()
}

/// The `.dat`'s rows, read in the order the file lists them.
fn dat_rows() -> Vec<[f32; 3]> {
    DAT.lines()
        .filter(|line| {
            let line = line.trim_start();
            !line.is_empty() && !line.starts_with('#') && !line.starts_with("3DLUTSIZE")
        })
        .map(|line| {
            let mut parts = line.split_whitespace();
            let (Some(r), Some(g), Some(b)) = (parts.next(), parts.next(), parts.next()) else {
                panic!("a .dat row has three channels: {line}");
            };
            [
                r.parse().expect("red"),
                g.parse().expect("green"),
                b.parse().expect("blue"),
            ]
        })
        .collect()
}

/// The probe's pixels as normalised codes.
fn probe_colours() -> Vec<[f32; 3]> {
    PROBE
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| {
            [
                f32::from(p[0]) / 255.0,
                f32::from(p[1]) / 255.0,
                f32::from(p[2]) / 255.0,
            ]
        })
        .collect()
}

/// The cube's own node values, read from its text in the order it lists them.
fn cube_nodes(text: &str) -> Vec<[f32; 3]> {
    text.lines()
        .filter_map(|line| {
            // A value line begins with a number; every other line in the file
            // is a directive or a comment.
            if !line
                .as_bytes()
                .first()
                .is_some_and(|c| c.is_ascii_digit() || *c == b'-' || *c == b'.')
            {
                return None;
            }
            let mut parts = line.split_whitespace();
            let (Some(r), Some(g), Some(b)) = (parts.next(), parts.next(), parts.next()) else {
                return None;
            };
            Some([
                r.parse().expect("red"),
                g.parse().expect("green"),
                b.parse().expect("blue"),
            ])
        })
        .collect()
}

/// A node's coordinate is a multiple of `1/(size-1)`, which binary floating
/// point holds exactly, so nearest sampling lands on the node itself.
#[test]
fn the_text_and_the_sampler_agree_on_which_node_is_which() {
    let lut = Lut::from_cube(CUBE).expect("a written cube is a cube");
    let nodes = cube_nodes(CUBE);
    assert_eq!(nodes.len(), SIZE.pow(3));
    for (index, node) in nodes.iter().enumerate() {
        // The file lists red fastest, then green, then blue.
        let r = (index % SIZE) as f32 / (SIZE - 1) as f32;
        let g = (index / SIZE % SIZE) as f32 / (SIZE - 1) as f32;
        let b = (index / (SIZE * SIZE)) as f32 / (SIZE - 1) as f32;
        let got = lut.sample([r, g, b], Interpolation::Nearest);
        for ch in 0..3 {
            assert!(
                (got[ch] - node[ch]).abs() < 1e-6,
                "node {index} channel {ch}: {got:?} against {node:?}"
            );
        }
    }
}

/// An identity cube is the case whose answer is known without a second
/// implementation: a linear ramp interpolated over a linear ramp is the ramp,
/// so a round trip through 8-bit codes has to leave them where they are. Only
/// the interpolations that read between nodes are exact; nearest snaps.
#[test]
fn an_identity_cube_leaves_the_codes_as_they_came() {
    let lut = Lut::Three(Lut3d::identity(33));
    for pixel in PROBE.as_chunks::<3>().0 {
        let codes = [pixel[0], pixel[1], pixel[2]];
        let normalised = [
            f32::from(codes[0]) / 255.0,
            f32::from(codes[1]) / 255.0,
            f32::from(codes[2]) / 255.0,
        ];
        for interp in [Interpolation::Trilinear, Interpolation::Tetrahedral] {
            let got = lut.sample(normalised, interp);
            for ch in 0..3 {
                assert_eq!(
                    (got[ch] * 255.0).round() as u8,
                    codes[ch],
                    "{interp:?} moved {ch} of {codes:?}"
                );
            }
        }
    }
}

/// The comparison this fixture exists for: same cube, same probe, same three
/// interpolation modes, and the two implementations within one code of each
/// other on all 12 288 channels — while `ffmpeg`'s byte is always the floor of
/// Fvid's own float result, which is the whole of the difference.
#[test]
fn each_interpolation_walks_with_ffmpeg() {
    let lut = Lut::from_cube(CUBE).expect("a written cube is a cube");
    // Of the 12 288 channels in the probe, these are the ones the two
    // implementations round differently rather than agree on.
    for (mode, interp, rounded_differently) in [
        ("nearest", Interpolation::Nearest, 4_826usize),
        ("trilinear", Interpolation::Trilinear, 5_354),
        ("tetrahedral", Interpolation::Tetrahedral, 5_298),
    ] {
        let theirs = reference(mode);
        assert_eq!(theirs.len(), PROBE.len());
        let mut differences = 0usize;
        for (i, pixel) in PROBE.as_chunks::<3>().0.iter().enumerate() {
            let codes = [
                f32::from(pixel[0]) / 255.0,
                f32::from(pixel[1]) / 255.0,
                f32::from(pixel[2]) / 255.0,
            ];
            let exact = lut.sample(codes, interp);
            for ch in 0..3 {
                let mine = (exact[ch] * 255.0).round() as i32;
                let other = i32::from(theirs[i * 3 + ch]);
                let delta = (mine - other).abs();
                assert!(delta <= 1, "{mode} channel {ch} moved {delta} codes");
                differences += usize::from(delta == 1);
                assert_eq!(
                    other,
                    (exact[ch] * 255.0).floor() as i32,
                    "{mode}: the gap is meant to be rounding alone"
                );
            }
        }
        assert_eq!(
            differences, rounded_differently,
            "{mode}: the count that differs by one code"
        );
    }
}

/// The same look written as a `.3dl` rather than a `.cube`: the file lists its
/// grid with the blue axis fastest and carries 12-bit integers where a cube
/// carries floats, and Fvid's sampler still meets `ffmpeg` one code at most on
/// every one of the probe's 12 288 channels, in each of the three modes.
///
/// The residue is wider than the cube's — 6 564 channels moved by one code at
/// nearest against 4 826 — because the nodes themselves are quantised to 1/4095,
/// so a value on a rounding boundary is decided by six bits the picture does not
/// have. It stays one-sided, which is the part that matters: `ffmpeg` is never
/// the higher byte, where a disagreement about the grid or the interpolation
/// would put it on both sides.
#[test]
fn a_3dl_of_the_same_look_walks_with_ffmpeg() {
    let lut = Lut::from_text(THREE_DL).expect("a written .3dl is a .3dl");
    let colours = probe_colours();
    for (mode, interp, rounded_differently) in [
        ("nearest", Interpolation::Nearest, 6_564usize),
        ("trilinear", Interpolation::Trilinear, 6_635),
        ("tetrahedral", Interpolation::Tetrahedral, 6_621),
    ] {
        let theirs = reference_3dl(mode);
        assert_eq!(theirs.len(), PROBE.len());
        let mut differences = 0usize;
        let mut theirs_above = 0usize;
        for (i, rgb) in colours.iter().enumerate() {
            let exact = lut.sample(*rgb, interp);
            for ch in 0..3 {
                let mine = (exact[ch] * 255.0).round() as i32;
                let other = i32::from(theirs[i * 3 + ch]);
                let delta = (mine - other).abs();
                assert!(
                    delta <= 1,
                    "{mode}: pixel {i} channel {ch} moved {delta} codes, over {mine} against {other}"
                );
                differences += usize::from(delta == 1);
                theirs_above += usize::from(mine < other);
            }
        }
        assert_eq!(
            differences, rounded_differently,
            "{mode}: the count that differs by one code"
        );
        assert_eq!(
            theirs_above, 0,
            "{mode}: `ffmpeg` came out higher than Fvid, which truncation does not explain"
        );
    }
}

/// Which axis a `.3dl` file varies fastest is not a detail a tolerance can
/// absorb: read the same rows as red-fastest — Fvid's own table layout, and the
/// order a `.cube` lists — and the look is transposed between red and blue. On
/// this probe that is 9 687 of 12 288 channels moved by more than one code, the
/// worst of them by the full 255, against the 6 564 the rounding leaves.
///
/// `ffmpeg`'s bytes are the arbiter, and they are blue-fastest: given a grid
/// whose every node stores its own file position, its nearest output selects
/// node `b + size·(g + size·r)` for all 4 913 of them — which is also what
/// Autodesk's own reader says the format stores.
#[test]
fn a_3dl_read_red_fastest_is_a_different_look_not_a_rounding_gap() {
    let blind = Lut::Three(Lut3d {
        size: SIZE,
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
        data: three_dl_rows(),
    });
    let theirs = reference_3dl("nearest");
    let mut misplaced = 0usize;
    let mut worst = 0i32;
    for (i, rgb) in probe_colours().iter().enumerate() {
        let exact = blind.sample(*rgb, Interpolation::Nearest);
        for ch in 0..3 {
            let delta = ((exact[ch] * 255.0).round() as i32 - i32::from(theirs[i * 3 + ch])).abs();
            misplaced += usize::from(delta > 1);
            worst = worst.max(delta);
        }
    }
    assert_eq!(
        misplaced, 9_687,
        "the wrong axis order moved this many channels by more than one code"
    );
    assert_eq!(
        worst, 255,
        "the wrong order's worst channel moved {worst} codes"
    );
}

/// The `.3dl` writers actually in use — Autodesk's export, Synthetic Aperture's
/// Color Finesse, Photoshop's lookup plugin — declare the grid with a mesh line
/// instead of a bare size: one 10-bit input code per node along an axis, so
/// `0 64 128 … 960 1023` for a 17-grid. `ffmpeg` reads that line as the size
/// declaration and nothing else, and re-running its three commands on this
/// twin gives the reference bytes above unchanged (README, same section), so
/// the same comparison holds — and the table Fvid builds has to be identical to
/// the one it builds from the size line, not merely close.
#[test]
fn a_3dl_that_declares_its_mesh_gives_ffmpeg_the_same_table() {
    let mesh = (0..SIZE)
        .map(|i| {
            let code = (i as f32 * 1023.0 / (SIZE - 1) as f32).round() as usize;
            code.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let body = THREE_DL.split_once('\n').expect("the fixture has rows").1;
    let declared =
        Lut::from_text(&format!("{mesh}\n{body}")).expect("a mesh-declared .3dl is read as a .3dl");
    let sized = Lut::from_text(THREE_DL).expect("the size line is read too");
    let (Lut::Three(declared), Lut::Three(sized)) = (&declared, &sized) else {
        panic!("a .3dl grid is a 3D LUT");
    };
    assert_eq!(declared.size, sized.size);
    assert_eq!(
        declared.data, sized.data,
        "the mesh line was read as something other than the size declaration"
    );

    let colours = probe_colours();
    for (mode, interp, rounded_differently) in [
        ("nearest", Interpolation::Nearest, 6_564usize),
        ("trilinear", Interpolation::Trilinear, 6_635),
        ("tetrahedral", Interpolation::Tetrahedral, 6_621),
    ] {
        let theirs = reference_3dl(mode);
        let mut differences = 0usize;
        let mut theirs_above = 0usize;
        for (i, rgb) in colours.iter().enumerate() {
            let exact = declared.sample(*rgb, interp);
            for ch in 0..3 {
                let mine = (exact[ch] * 255.0).round() as i32;
                let other = i32::from(theirs[i * 3 + ch]);
                let delta = (mine - other).abs();
                assert!(
                    delta <= 1,
                    "{mode}: pixel {i} channel {ch} moved {delta} codes"
                );
                differences += usize::from(delta == 1);
                theirs_above += usize::from(mine < other);
            }
        }
        assert_eq!(
            differences, rounded_differently,
            "{mode}: the count that differs by one code"
        );
        assert_eq!(
            theirs_above, 0,
            "{mode}: `ffmpeg` came out higher than Fvid"
        );
    }
}

/// The third transcription of the same look, and the one whose axis order the
/// format itself never states: an Iridas `.dat` is a `3DLUTSIZE` line and then
/// the grid, with nothing marking which channel the rows walk. `ffmpeg`'s
/// `parse_dat` answers it only in code, by putting its innermost loop in the
/// ones place of its own slot — blue fastest, as in a `.3dl`. The fixture was
/// therefore written out in all six assignments of the file's digit places to
/// red, green and blue and given to `ffmpeg`; the blue-fastest one is the only
/// file whose output matches its own `.cube` byte for byte on all 12 288 probe
/// channels in every mode, and the table Fvid builds from it is then the same
/// 4 913 nodes the cube reader builds, with the same residue against `ffmpeg` as
/// the cube itself has: rounding, one code, one-sided.
#[test]
fn a_dat_of_the_same_look_walks_with_ffmpeg() {
    let lut = Lut::from_text(DAT).expect("a written .dat is a .dat");
    let cube = Lut::from_cube(CUBE).expect("a written cube is a cube");
    let (Lut::Three(dat), Lut::Three(cub)) = (&lut, &cube) else {
        panic!("both fixtures are grids");
    };
    assert_eq!(dat.size, cub.size);
    assert_eq!(dat.domain_min, cub.domain_min);
    assert_eq!(dat.domain_max, cub.domain_max);
    assert_eq!(
        dat.data, cub.data,
        "the .dat and the .cube of one look built different tables"
    );

    let colours = probe_colours();
    for (mode, interp, rounded_differently) in [
        ("nearest", Interpolation::Nearest, 4_826usize),
        ("trilinear", Interpolation::Trilinear, 5_354),
        ("tetrahedral", Interpolation::Tetrahedral, 5_298),
    ] {
        let theirs = reference(mode);
        let mut differences = 0usize;
        for (i, rgb) in colours.iter().enumerate() {
            let exact = lut.sample(*rgb, interp);
            for ch in 0..3 {
                let mine = (exact[ch] * 255.0).round() as i32;
                let other = i32::from(theirs[i * 3 + ch]);
                let delta = (mine - other).abs();
                assert!(
                    delta <= 1,
                    "{mode}: pixel {i} channel {ch} moved {delta} codes, {mine} against {other}"
                );
                differences += usize::from(delta == 1);
                assert_eq!(
                    other,
                    (exact[ch] * 255.0).floor() as i32,
                    "{mode}: the gap is meant to be rounding alone"
                );
            }
        }
        assert_eq!(
            differences, rounded_differently,
            "{mode}: the count that differs by one code"
        );
    }
}

/// Which axis a `.dat` runs fastest is worth 9 696 channels of the look: taken
/// in the order a `.cube` lists them — Fvid's own table layout — the same 4 913
/// rows become a grid transposed between red and blue, moving that many of the
/// probe's 12 288 channels by more than one code at nearest, the worst by the
/// full 255, against the 4 826 the rounding leaves. Trilinear and tetrahedral
/// are worse again (10 061 and 10 056), because mixing transposed nodes spreads
/// the error instead of moving it in blocks.
#[test]
fn a_dat_read_red_fastest_is_a_different_look_not_a_rounding_gap() {
    let rows = dat_rows();
    assert_eq!(rows.len(), SIZE.pow(3));
    let blind = Lut::Three(Lut3d {
        size: SIZE,
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
        data: rows,
    });
    let colours = probe_colours();
    for (mode, interp, misplaced) in [
        ("nearest", Interpolation::Nearest, 9_696usize),
        ("trilinear", Interpolation::Trilinear, 10_061),
        ("tetrahedral", Interpolation::Tetrahedral, 10_056),
    ] {
        let theirs = reference(mode);
        let mut moved = 0usize;
        let mut worst = 0i32;
        for (i, rgb) in colours.iter().enumerate() {
            let exact = blind.sample(*rgb, interp);
            for ch in 0..3 {
                let delta =
                    ((exact[ch] * 255.0).round() as i32 - i32::from(theirs[i * 3 + ch])).abs();
                moved += usize::from(delta > 1);
                worst = worst.max(delta);
            }
        }
        assert_eq!(
            moved, misplaced,
            "{mode}: the wrong axis order moved this many channels by more than one code"
        );
        assert_eq!(
            worst, 255,
            "{mode}: the wrong order's worst channel moved {worst} codes"
        );
    }
}

/// Real grading LUTs overshoot: the six `.cube` exports in the D-LUT set run to
/// 1.07 and below 0, and a log-to-linear table runs to 12. `ffmpeg` reads such a
/// file as written — it keeps the node value and only saturates the byte it
/// emits, which a size-2 cube with a red node at 1.5 shows directly: its
/// trilinear output at mid-grey is red 192, i.e. half of 1.5, where a reader
/// that clipped the node at parse time can only ever return 128. The same
/// instrument holds for `.3dl` (README, "Domains and out-of-range values").
///
/// So the nodes stay as they are and the ends are decided at the code. Held to
/// that, Fvid meets `ffmpeg` within one code on all 12 288 channels of the probe
/// in each mode, and the residue is still rounding alone: `ffmpeg`'s byte is the
/// saturated floor of Fvid's float.
#[test]
fn a_cube_that_overshoots_the_display_range_keeps_its_nodes() {
    let lut = Lut::from_cube(OVER).expect("an overshooting cube is still a cube");
    let Lut::Three(cube) = &lut else {
        panic!("a 17-grid is a 3D LUT");
    };
    assert_eq!(cube.data.len(), SIZE.pow(3));
    // The table's own layout: node (r, g, b) at `r + size·(g + size·b)`.
    let corner = |r: usize, g: usize, b: usize| cube.data[r + SIZE * (g + SIZE * b)];
    for (node, want) in [
        ((0, 0, 0), [-0.0227, -0.0107, -0.0038]),
        ((16, 16, 16), [1.0695, 1.0588, 1.0192]),
    ] {
        let got = corner(node.0, node.1, node.2);
        for ch in 0..3 {
            assert!(
                (got[ch] - want[ch]).abs() < 1e-6,
                "node {node:?} channel {ch} came back {got:?}, the file says {want:?}"
            );
        }
    }

    let colours = probe_colours();
    for (mode, interp, rounded_differently) in [
        ("nearest", Interpolation::Nearest, 6_400usize),
        ("trilinear", Interpolation::Trilinear, 5_888),
        ("tetrahedral", Interpolation::Tetrahedral, 5_888),
    ] {
        let theirs = reference_over(mode);
        assert_eq!(theirs.len(), PROBE.len());
        let mut differences = 0usize;
        for (i, rgb) in colours.iter().enumerate() {
            let exact = cube.sample(*rgb, interp);
            for ch in 0..3 {
                let float = exact[ch] * 255.0;
                let mine = float.round().clamp(0.0, 255.0) as i32;
                let other = i32::from(theirs[i * 3 + ch]);
                let delta = (mine - other).abs();
                assert!(
                    delta <= 1,
                    "{mode}: pixel {i} channel {ch} moved {delta} codes, {mine} against {other}"
                );
                differences += usize::from(delta == 1);
                assert_eq!(
                    other,
                    float.floor().clamp(0.0, 255.0) as i32,
                    "{mode}: the gap is meant to be rounding alone"
                );
            }
        }
        assert_eq!(
            differences, rounded_differently,
            "{mode}: the count that differs by one code"
        );
    }
}

/// The clamp is not a rounding question. Read the identical nodes with the ends
/// cut off before they reach the table — what `from_cube` did — and 384 of the
/// probe's 12 288 channels move by more than one code through the interpolators
/// that read between nodes, the worst by four, and 256 channels that should
/// sit hard at 0 or 255 come back inside the range instead. Nearest is untouched
/// because it never mixes two nodes, which is why the defect only shows up in a
/// graded picture rather than in a node-by-node check.
#[test]
fn clamping_a_cube_at_parse_is_a_different_look_not_a_rounding_gap() {
    let blind = Lut3d {
        size: SIZE,
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
        data: cube_nodes(OVER)
            .into_iter()
            .map(|n| {
                [
                    n[0].clamp(0.0, 1.0),
                    n[1].clamp(0.0, 1.0),
                    n[2].clamp(0.0, 1.0),
                ]
            })
            .collect(),
    };
    let colours = probe_colours();
    for mode in ["trilinear", "tetrahedral"] {
        let theirs = reference_over(mode);
        let interp = if mode == "trilinear" {
            Interpolation::Trilinear
        } else {
            Interpolation::Tetrahedral
        };
        let mut misplaced = 0usize;
        let mut worst = 0i32;
        let mut short_of_white = 0usize;
        let mut short_of_black = 0usize;
        for (i, rgb) in colours.iter().enumerate() {
            let exact = blind.sample(*rgb, interp);
            for ch in 0..3 {
                let mine = (exact[ch] * 255.0).round().clamp(0.0, 255.0) as i32;
                let other = i32::from(theirs[i * 3 + ch]);
                let delta = (mine - other).abs();
                misplaced += usize::from(delta > 1);
                worst = worst.max(delta);
                short_of_white += usize::from(other == 255 && mine != 255);
                short_of_black += usize::from(other == 0 && mine != 0);
            }
        }
        assert_eq!(
            misplaced, 384,
            "{mode}: the clamp moved this many channels by more than one code"
        );
        assert_eq!(
            worst, 4,
            "{mode}: the clamp's worst channel moved {worst} codes"
        );
        assert_eq!(
            short_of_white, 192,
            "{mode}: channels `ffmpeg` clips to white that the clamped table leaves short"
        );
        assert_eq!(
            short_of_black, 64,
            "{mode}: channels `ffmpeg` clips to black that the clamped table leaves short"
        );
    }
}

/// The same 17-grid with the sections a grading suite writes in front of it: a
/// `LUT_PRELUT_1D_SIZE` curve, a `LUT_PRELUT_3D_SIZE` grid, a
/// `LUT_1D_SHAPER_SIZE` curve and the `LUT_TYPE`, `VERTEX_FORMAT`,
/// `NUM_SAMPLES` and `LUT_3D_OUTPUT_RANGE` keys, behind two comment lines.
/// `ffmpeg lut3d` opens the file and moves nothing: the bytes it wrote for it are
/// the bytes it wrote for the bare grid, byte for byte in all three modes, so the
/// shaper that maps white to black leaves the picture exactly as white.
///
/// OpenColorIO's Resolve reader refuses the same file outright — `ociochecklut`
/// reports `At line (1): 'LUT_TYPE 3D'. Malformed color triples specified`, and
/// the same for each of the other keys — so the tolerance here follows FFmpeg,
/// the reader this repository qualifies against, and the record of what each key
/// is worth sits in `tests/fixtures/lut/README.md`. Refusing a key this reader
/// has no clause for would lock out a file whose colour is settled; a section's
/// rows are counted out instead, so they cannot land in the grid's own bucket and
/// shift the look it carries.
#[test]
fn a_cube_with_a_pre_lut_and_vendor_keys_walks_with_ffmpeg() {
    let with = Lut::from_cube(PRELUTED).expect("a grid with sections ahead of it is still a cube");
    let plain = Lut::from_cube(CUBE).expect("a written cube is a cube");
    assert_eq!(with, plain, "the sections moved what the sampler reads");
    // The player and `--lut` hand the file to the sniffing entry, not to
    // `from_cube`, so the same file has to be read as a cube there too.
    let sniffed = Lut::from_text(PRELUTED).expect("the sniffer reads a preluted file as a cube");
    assert_eq!(sniffed, plain, "the sniffer and the cube reader agree");

    let colours = probe_colours();
    for (mode, interp, rounded_differently) in [
        ("nearest", Interpolation::Nearest, 4_826usize),
        ("trilinear", Interpolation::Trilinear, 5_354),
        ("tetrahedral", Interpolation::Tetrahedral, 5_298),
    ] {
        let theirs = reference_preluted(mode);
        assert_eq!(
            theirs,
            reference(mode),
            "{mode}: the oracle's own answer for the two files"
        );
        let mut differences = 0usize;
        for (i, rgb) in colours.iter().enumerate() {
            let exact = with.sample(*rgb, interp);
            for ch in 0..3 {
                let mine = (exact[ch] * 255.0).round() as i32;
                let other = i32::from(theirs[i * 3 + ch]);
                let delta = (mine - other).abs();
                assert!(delta <= 1, "{mode} channel {ch} moved {delta} codes");
                differences += usize::from(delta == 1);
            }
        }
        assert_eq!(
            differences, rounded_differently,
            "{mode}: the count that differs by one code"
        );
    }
}

/// `LUT_3D_INPUT_RANGE 0.0 0.5` is the grid's input domain in a scalar pair, and
/// OCIO reads it as one: over the 17-grid of `grade-17.cube` with its DOMAIN lines
/// taken out, `ociochecklut` answers 0.25 with 0.251519, and answers it with
/// 0.531026 once that pair is written ahead of the grid — the same file's own
/// answer at 0.5, so half the declared range is the whole grid. `DOMAIN_MAX 0.5`
/// gives the identical 0.531026, which is the disagreement FFmpeg has with this
/// reader about `DOMAIN_MIN`/`DOMAIN_MAX` too, and which the domain has followed
/// OCIO on since it was first read. The 2-grid below is that measurement written
/// down in the smallest file that states it, and the tables beside it are the
/// same key over a 1D table, where OCIO returns 0.5 for 0.25 and the grid's own
/// 0.25 for a pair written under the other table's key.
#[test]
fn a_cube_naming_its_input_range_grids_the_domain() {
    let text = "LUT_3D_SIZE 2\nLUT_3D_INPUT_RANGE 0.0 0.5\n\
                0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    let lut = Lut::from_cube(text).expect("a grid with its range stated is a cube");
    let Lut::Three(cube) = &lut else {
        panic!("a size line makes it a grid");
    };
    assert_eq!(cube.domain_min, [0.0; 3]);
    assert_eq!(cube.domain_max, [0.5; 3]);
    assert_eq!(lut.sample([0.25; 3], Interpolation::Trilinear), [0.5; 3]);
    // A pair under the other table's key is not this grid's range, and OCIO says
    // so with the same 0.25 it returns for a file that states no range at all.
    let other = text.replace("LUT_3D_INPUT_RANGE", "LUT_1D_INPUT_RANGE");
    let lut = Lut::from_cube(&other).expect("the 1D key does not close a grid");
    assert_eq!(lut.sample([0.25; 3], Interpolation::Trilinear), [0.25; 3]);
    // The same key over a 1D table is that table's domain, pinned on OCIO's two
    // answers for it: 0.25 of a 0…0.5 range is the middle entry, 0.5 is the top.
    let one = "LUT_1D_SIZE 3\nLUT_1D_INPUT_RANGE 0.0 0.5\n0.0 0.0 0.0\n0.5 0.5 0.5\n1.0 1.0 0.0\n";
    let lut = Lut::from_cube(one).expect("a 1D table with its range stated");
    let Lut::One(table) = &lut else {
        panic!("a size line makes it a table");
    };
    assert_eq!(table.domain_max, [0.5; 3]);
    assert_eq!(lut.sample([0.25; 3], Interpolation::Trilinear), [0.5; 3]);
    assert_eq!(
        lut.sample([0.5; 3], Interpolation::Trilinear),
        [1.0, 1.0, 0.0]
    );
    // The same over a ramp long enough to place the answer inside the table rather
    // than on an entry: OCIO returns 0.5 for 0.25 and 1.0 for 0.5 against a 17-entry
    // ramp, which is the ramp read at half its own length.
    let mut ramp = String::from("LUT_1D_SIZE 17\nLUT_1D_INPUT_RANGE 0.0 0.5\n");
    for i in 0..17 {
        let v = i as f32 / 16.0;
        ramp.push_str(&format!("{v:.6} {v:.6} {v:.6}\n"));
    }
    let lut = Lut::from_cube(&ramp).expect("a long table with its range stated");
    assert_eq!(lut.sample([0.25; 3], Interpolation::Trilinear), [0.5; 3]);
    assert_eq!(lut.sample([0.5; 3], Interpolation::Trilinear), [1.0; 3]);
}
