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
const THREE_DL: &str = include_str!("fixtures/lut/grade-17.3dl");
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

/// The reference `ffmpeg` wrote for the `.3dl` twin of the same look.
fn reference_3dl(mode: &str) -> Vec<u8> {
    match mode {
        "nearest" => include_bytes!("fixtures/lut/ffmpeg-3dl-nearest.rgb").to_vec(),
        "trilinear" => include_bytes!("fixtures/lut/ffmpeg-3dl-trilinear.rgb").to_vec(),
        "tetrahedral" => include_bytes!("fixtures/lut/ffmpeg-3dl-tetrahedral.rgb").to_vec(),
        other => unreachable!("no .3dl reference for {other}"),
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
fn nodes() -> Vec<[f32; 3]> {
    CUBE.lines()
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
    let nodes = nodes();
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
