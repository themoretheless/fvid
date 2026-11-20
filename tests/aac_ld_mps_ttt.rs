use fvid_media::owned_aac::{
    aac_ld_mps_entropy::Kind,
    aac_ld_mps_prepare::dequantize,
    aac_ld_mps_ttt::{from_indices, inversion_weights, invert, transform},
    aac_sbr_qmf::Complex,
};
use serde_json::Value;
fn fixture() -> (Value, Vec<u8>) {
    let r = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-ttt");
    (
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap(),
        std::fs::read(r.join("expected.bin")).unwrap(),
    )
}
fn integer(c: &Value, i: usize) -> i16 {
    c[i].as_i64().unwrap() as i16
}
fn expected(blob: &[u8], offset: usize, index: usize) -> f64 {
    let o = offset + index * 8;
    f64::from_le_bytes(blob[o..o + 8].try_into().unwrap())
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-12, "{a} != {b}");
}
#[test]
fn all_ttt_parameter_pairs_and_coding_mode_aliases_match_decimal_oracle() {
    let (m, blob) = fixture();
    assert_eq!(m["cases"].as_array().unwrap().len(), 40937);
    for c in m["cases"].as_array().unwrap() {
        let mode = integer(c, 0) as u8;
        let args = (
            integer(c, 1),
            integer(c, 2),
            integer(c, 3),
            c[4].as_bool().unwrap(),
        );
        let offset = c[5].as_u64().unwrap() as usize;
        let a = from_indices(mode, args.0, args.1, args.2, args.3).unwrap();
        assert_eq!(
            a,
            from_indices(mode + 1, args.0, args.1, args.2, args.3).unwrap()
        );
        for (i, &v) in a.iter().flatten().enumerate() {
            close(v, expected(&blob, offset, i));
        }
    }
}
fn multiply(a: [[Complex; 2]; 2], b: [[Complex; 2]; 2]) -> [[Complex; 2]; 2] {
    std::array::from_fn(|r| {
        std::array::from_fn(|c| {
            let mut z = Complex::default();
            for k in 0..2 {
                z.re += a[r][k].re * b[k][c].re - a[r][k].im * b[k][c].im;
                z.im += a[r][k].re * b[k][c].im + a[r][k].im * b[k][c].re;
            }
            z
        })
    })
}
#[test]
fn complex_inversion_matches_decimal_adjugate_and_both_inverse_products() {
    let (m, blob) = fixture();
    assert_eq!(m["inversions"].as_array().unwrap().len(), 7688);
    for c in m["inversions"].as_array().unwrap() {
        let mode = integer(c, 0) as u8;
        let ott = [
            dequantize(Kind::Cld, integer(c, 1)).unwrap(),
            dequantize(Kind::Cld, integer(c, 2)).unwrap(),
        ];
        let kind = if mode < 2 { Kind::Cpc } else { Kind::Cld };
        let parameters = [
            dequantize(kind, integer(c, 3)).unwrap(),
            dequantize(kind, integer(c, 4)).unwrap(),
        ];
        let offset = c[5].as_u64().unwrap() as usize;
        let weights = inversion_weights(mode, ott, parameters).unwrap();
        assert_eq!(
            weights,
            inversion_weights(mode + 1, ott, parameters).unwrap()
        );
        for i in 0..2 {
            close(weights[i], expected(&blob, offset, i));
        }
        let h = invert(weights).unwrap();
        for (i, z) in h.inverse.iter().flatten().enumerate() {
            close(z.re, expected(&blob, offset, 2 + i * 2));
            close(z.im, expected(&blob, offset, 3 + i * 2));
        }
        for product in [
            multiply(h.forward, h.inverse),
            multiply(h.inverse, h.forward),
        ] {
            for r in 0..2 {
                for c in 0..2 {
                    close(product[r][c].re, if r == c { 1.0 } else { 0.0 });
                    close(product[r][c].im, 0.0);
                }
            }
        }
    }
}
#[test]
fn zero_coherence_is_a_specific_singularity_only_outside_residual_region() {
    for a in -20..=30 {
        for b in -20..=30 {
            for mode in 0..=1 {
                let error = from_indices(mode, a, b, 5, false).unwrap_err().to_string();
                assert!(error.contains("singular") && error.contains("coherence"));
                assert!(from_indices(mode, a, b, 5, true).is_ok());
            }
        }
    }
    let a = transform(0, 1.0, 1.0, 1.0, false).unwrap();
    assert_eq!(a[0], [1.0, 0.0, 1.0 / 3.0]);
    assert_eq!(a[1], [0.0, 1.0, 1.0 / 3.0]);
    // sqrt(2)/3, rounded once from the 80-digit real value. Computing
    // f64 sqrt(2) and then dividing by three introduces a second rounding.
    assert_eq!(a[2], [0.0, 0.0, -0.4714045207910317]);
    let zero = invert([0.0, 0.0]).unwrap();
    assert_eq!(zero.forward, zero.inverse);
    for weights in [[1.0, 1.0], [0.0, 1.0], [1.0, 0.0]] {
        assert!(invert(weights).is_ok());
    }
}
#[test]
fn invalid_modes_indices_and_physical_values_are_refused() {
    for mode in 6..=9 {
        assert!(from_indices(mode, 0, 0, 0, false).is_err());
    }
    assert!(from_indices(0, -21, 0, 0, false).is_err());
    assert!(from_indices(2, 16, 0, 0, false).is_err());
    assert!(from_indices(0, 0, 0, 8, false).is_err());
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(transform(0, value, 0.0, 1.0, false).is_err());
        assert!(transform(2, 0.0, value, 1.0, false).is_err());
        assert!(invert([value, 0.0]).is_err());
        assert!(inversion_weights(0, [0.0, value], [0.0, 0.0]).is_err());
    }
    assert!(transform(0, 3.1, 0.0, 1.0, false).is_err());
    assert!(transform(2, 151.0, 0.0, 1.0, false).is_err());
    assert!(invert([-0.1, 0.0]).is_err());
    assert!(invert([0.0, 1.1]).is_err());
}
