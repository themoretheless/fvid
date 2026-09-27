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
