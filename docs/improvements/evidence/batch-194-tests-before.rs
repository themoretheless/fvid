#![cfg(feature = "player")]

use std::path::PathBuf;

use fvid::color::grade::{Grade, Settings};
use fvid::color::hdr::{ColourDescription, HdrMetadata, MasteringDisplay};
use fvid::color::lut::{Interpolation, Lut};
use fvid::color::tonemap::DisplayTarget;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("color")
        .join(name)
}

#[test]
fn refusal_3dl_rejects_non_finite_node_and_mesh_values() {
    let text = std::fs::read_to_string(fixture_path("nan_node.3dl")).unwrap();
    assert!(
        Lut::from_3dl(&text).is_err(),
        "a .3dl containing NaN node values must be rejected"
    );
    assert!(
        Lut::from_text(&text).is_err(),
        "Lut::from_text must reject a .3dl containing NaN node values"
    );
    assert!(
        Lut::from_3dl("2\ninf 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n").is_err(),
        "a .3dl containing inf node values must be rejected"
    );
    assert!(
        Lut::from_3dl("0 NaN\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n")
            .is_err(),
        "a .3dl with NaN in its mesh header line must be rejected"
    );
}

#[test]
fn refusal_cube_and_spi1d_reject_invalid_or_inverted_domains() {
    let cube_inverted = std::fs::read_to_string(fixture_path("inverted_domain.cube")).unwrap();
    assert!(
        Lut::from_cube(&cube_inverted).is_err(),
        "a .cube with DOMAIN_MIN >= DOMAIN_MAX must be rejected"
    );
    let cube_nan_domain = "LUT_3D_SIZE 2\nDOMAIN_MIN NaN 0 0\nDOMAIN_MAX 1 1 1\n\
        0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    assert!(
        Lut::from_cube(cube_nan_domain).is_err(),
        "a .cube with NaN DOMAIN_MIN must be rejected"
    );
    let cube_inverted_range = "LUT_3D_SIZE 2\nLUT_3D_INPUT_RANGE 1.0 0.0\n\
        0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    assert!(
        Lut::from_cube(cube_inverted_range).is_err(),
        "a .cube with inverted LUT_3D_INPUT_RANGE must be rejected"
    );
    let spi1d_inverted = std::fs::read_to_string(fixture_path("inverted_from.spi1d")).unwrap();
    assert!(
        Lut::from_spi1d(&spi1d_inverted).is_err(),
        "a .spi1d with From min >= max must be rejected"
    );
    let spi1d_nan = "Version 1\nFrom NaN 1.0\nLength 2\nComponents 1\n{\n0.0\n1.0\n}\n";
    assert!(
        Lut::from_spi1d(spi1d_nan).is_err(),
        "a .spi1d with NaN From bound must be rejected"
    );
}

#[test]
fn acceptance_1d_lut_is_identity_checks_all_entries_and_domains() {
    let text = std::fs::read_to_string(fixture_path("spike_between_probes_1d.cube")).unwrap();
    let lut = Lut::from_cube(&text).expect("valid 33-entry 1D LUT");
    assert!(
        !lut.is_identity(0.5 / 255.0),
        "a 33-entry 1D LUT with a +0.5 spike at index 1 (between 1/16th probes) is not an identity"
    );

    let sdr = ColourDescription {
        primaries: 1,
        transfer: 1,
        matrix: 1,
        full_range: false,
    };
    let grade = Grade::new(
        sdr,
        &HdrMetadata::default(),
        Settings::video(DisplayTarget::sdr(240.0)),
        Some(lut),
    );
    assert!(
        !grade.is_identity(),
        "a grade carrying a non-identity 1D LUT must not report is_identity()"
    );
    let mut pixel = [8u8, 8, 8]; // 8/255 ≈ 1/32 = 0.03137
    grade.apply(&mut pixel);
    assert!(
        pixel[0] > 100 && pixel[1] == 8 && pixel[2] == 8,
        "Grade::apply must not skip the 1D LUT spike at code 8: got {pixel:?}"
    );
}

#[test]
fn refusal_and_acceptance_mastering_display_rejects_zero_and_collinear_primaries() {
    let zero_mdcv = std::fs::read(fixture_path("zero_placeholder.mdcv")).unwrap();
    assert!(
        MasteringDisplay::from_payload(&zero_mdcv).is_none(),
        "an all-zero placeholder MDCV payload must be rejected"
    );
    let collinear_mdcv = std::fs::read(fixture_path("collinear_primaries.mdcv")).unwrap();
    assert!(
        MasteringDisplay::from_payload(&collinear_mdcv).is_none(),
        "a collinear-primaries MDCV payload must be rejected"
    );
    assert!(
        MasteringDisplay::from_corners(
            (0.708, 0.292),
            (0.17, 0.797),
            (0.131, 0.046),
            (0.3127, 0.3290),
            100.0,
            200.0, // min_luminance > max_luminance
        )
        .is_none(),
        "min_luminance > max_luminance must be rejected"
    );

    // An HDR10 stream with an all-zero placeholder MDCV box must fall back cleanly
    // to the coded primaries/headroom without producing NaN in Grade.
    let hdr = HdrMetadata::from_mdcv(&zero_mdcv).unwrap();
    assert!(hdr.mastering.is_none());
    let pq_desc = ColourDescription {
        primaries: 9,
        transfer: 16,
        matrix: 9,
        full_range: false,
    };
    let grade = Grade::new(
        pq_desc,
        &hdr,
        Settings::video(DisplayTarget::sdr(100.0)),
        None,
    );
    let sample = grade.rgb([0.5, 0.5, 0.5]);
    assert!(
        sample.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)),
        "grading with a rejected placeholder MDCV must produce finite RGB, got {sample:?}"
    );
    let _ = Interpolation::Tetrahedral;
}
