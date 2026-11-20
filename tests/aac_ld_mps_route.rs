use fvid_media::owned_aac::{aac_ld_mps_route::Router, aac_sbr_qmf::Complex};
use serde_json::Value;
fn fixture() -> (Value, Vec<u8>) {
    let r = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-route");
    (
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap(),
        std::fs::read(r.join("expected.bin")).unwrap(),
    )
}
fn sample(kind: u8, t: usize, c: usize, q: usize) -> Complex {
    let (a, b, d) = match kind {
        0 => (
            ((t * 11 + c * 7 + q * 3) % 31) as f64 - 15.0,
            ((t * 5 + c * 13 + q * 7) % 29) as f64 - 14.0,
            16.0,
        ),
        1 => (
            ((t * 17 + c * 11 + q * 5) % 37) as f64 - 18.0,
            ((t * 13 + c * 3 + q * 17) % 43) as f64 - 21.0,
            32.0,
        ),
        _ => (
            ((t * 7 + c * 19 + q * 11) % 41) as f64 - 20.0,
            ((t * 3 + c * 17 + q * 13) % 47) as f64 - 23.0,
            64.0,
        ),
    };
    Complex {
        re: a / d,
        im: b / d,
    }
}
fn bytes(c: &Value, key: &str) -> Vec<u8> {
    c[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect()
}
#[test]
fn all_eight_trees_route_exact_dry_and_decimal_wet_expectations() {
    let (m, blob) = fixture();
    assert_eq!(m["cases"].as_array().unwrap().len(), 504);
    let mut scalars = 0;
    for c in m["cases"].as_array().unwrap() {
        let bins = c["bins"].as_u64().unwrap() as usize;
        let cutoffs = bytes(c, "cutoffs");
        let router = Router::new(
            c["tree"].as_u64().unwrap() as u8,
            c["bands"].as_u64().unwrap() as u8,
            bins,
            cutoffs.clone(),
            bytes(c, "modes"),
        )
        .unwrap();
        let channels = router.channels();
        let plane = |kind| {
            (0..2)
                .map(|t| {
                    (0..channels)
                        .map(|ch| (0..bins).map(|q| sample(kind, t, ch, q)).collect())
                        .collect()
                })
                .collect::<Vec<_>>()
        };
        let virtuals = plane(0);
        let decorrelated = plane(1);
        let residuals = (0..cutoffs.len())
            .map(|b| {
                if cutoffs[b] == 0 {
                    None
                } else {
                    Some(
                        (0..2)
                            .map(|t| (0..bins).map(|q| sample(2, t, b, q)).collect())
                            .collect(),
                    )
                }
            })
            .collect::<Vec<_>>();
        let a = router
            .process(&virtuals, &decorrelated, &residuals)
            .unwrap();
        let mut offset = c["offset"].as_u64().unwrap() as usize;
        for (plane, samples) in [&a.dry, &a.wet].into_iter().enumerate() {
            for value in samples.iter().flatten().flatten() {
                for actual in [value.re, value.im] {
                    let expected = f64::from_le_bytes(blob[offset..offset + 8].try_into().unwrap());
                    offset += 8;
                    scalars += 1;
                    if plane == 0 {
                        assert_eq!(actual, expected, "dry case {c}");
                    } else {
                        assert!(
                            (actual - expected).abs() < 1e-15,
                            "{actual} vs {expected}; case {c}"
                        );
                    }
                }
            }
        }
        for t in 0..2 {
            let res = residuals
                .iter()
                .map(|r| r.as_ref().map(|r| vec![r[t].clone()]))
                .collect::<Vec<_>>();
            let part = router
                .process(&virtuals[t..t + 1], &decorrelated[t..t + 1], &res)
                .unwrap();
            assert_eq!(part.dry[0], a.dry[t]);
            assert_eq!(part.wet[0], a.wet[t]);
        }
    }
    assert_eq!(scalars * 8, blob.len());
}
#[test]
fn ttt_mixture_uses_original_ott_decorrelation_before_residual_masks() {
    let router = Router::new(2, 4, 32, vec![0, 4, 4, 0], vec![0; 4]).unwrap();
    let virtuals = vec![vec![
        vec![
            Complex {
                re: 99.0,
                im: -99.0
            };
            32
        ];
        6
    ]];
    let mut decor = vec![vec![vec![Complex::default(); 32]; 6]];
    decor[0][3].fill(Complex { re: 1.0, im: 2.0 });
    decor[0][4].fill(Complex { re: 3.0, im: 5.0 });
    decor[0][5].fill(Complex { re: 7.0, im: 11.0 });
    let residuals = vec![
        None,
        Some(vec![vec![Complex { re: 0.25, im: 0.5 }; 32]]),
        Some(vec![vec![
            Complex {
                re: -0.25,
                im: -0.5
            };
            32
        ]]),
        None,
    ];
    let a = router.process(&virtuals, &decor, &residuals).unwrap();
    assert_eq!(a.wet[0][3][0], Complex::default());
    assert_eq!(a.wet[0][4][0], Complex::default());
    assert_eq!(a.dry[0][3][0], Complex { re: 0.25, im: 0.5 });
    let z = a.wet[0][5][0];
    assert!((z.re - (4.0 + 7.0 / 2.0f64.sqrt())).abs() < 2e-15);
    assert!((z.im - (7.0 + 11.0 / 2.0f64.sqrt())).abs() < 4e-15);
    // TTT residual belongs to M1; it must neither replace nor mute W[5].
    let ttt_residual_router = Router::new(2, 4, 32, vec![0, 4, 4, 4], vec![0; 4]).unwrap();
    assert_eq!(
        ttt_residual_router
            .process(&virtuals, &decor, &residuals)
            .unwrap(),
        a
    );
    let mut with_ttt = residuals.clone();
    with_ttt[3] = Some(vec![vec![
        Complex {
            re: 123.0,
            im: 456.0
        };
        32
    ]]);
    assert_eq!(
        ttt_residual_router
            .process(&virtuals, &decor, &with_ttt)
            .unwrap(),
        a
    );
    assert!(
        a.wet[0][..3]
            .iter()
            .flatten()
            .all(|z| *z == Complex::default())
    );
}
#[test]
fn missing_residual_nonfinite_geometry_and_overflow_never_change_inputs() {
    let router = Router::new(7, 4, 32, vec![2], vec![]).unwrap();
    let input = vec![vec![vec![Complex::default(); 32]; 2]; 2];
    let saved = input.clone();
    assert!(
        router
            .process(&input, &input, &[None])
            .unwrap_err()
            .to_string()
            .contains("missing")
    );
    let res = vec![Some(vec![vec![Complex::default(); 32]; 2])];
    assert!(router.process(&input, &input, &res).is_ok());
    assert_eq!(input, saved);
    let mut bad = input.clone();
    bad[1][1][31].im = f64::NAN;
    assert!(router.process(&input, &bad, &res).is_err());
    let mut res_bad = res.clone();
    res_bad[0].as_mut().unwrap()[1].pop();
    assert!(router.process(&input, &input, &res_bad).is_err());
    let mut bad = input.clone();
    bad[1].pop();
    assert!(router.process(&bad, &input, &res).is_err());
    let router = Router::new(2, 4, 32, vec![0; 4], vec![0; 4]).unwrap();
    let huge = vec![vec![
        vec![
            Complex {
                re: f64::MAX,
                im: 0.0
            };
            32
        ];
        6
    ]];
    assert!(
        router
            .process(&huge, &huge, &[None, None, None, None])
            .unwrap_err()
            .to_string()
            .contains("overflowing")
    );
    let empty = router.process(&[], &[], &[None, None, None, None]).unwrap();
    assert!(empty.dry.is_empty() && empty.wet.is_empty());
    assert!(
        Router::new(0, 4, 32, vec![0, 0, 0, 0, 1], vec![])
            .unwrap_err()
            .to_string()
            .contains("LFE residual")
    );
    assert!(Router::new(2, 4, 32, vec![0; 4], vec![6; 4]).is_err());
    assert!(Router::new(7, 4, 32, vec![5], vec![]).is_err());
    assert!(Router::new(8, 4, 32, vec![], vec![]).is_err());
    assert!(Router::new(2, 4, 32, vec![0; 4], vec![]).is_err());
}
