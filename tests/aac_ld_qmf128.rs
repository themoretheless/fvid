use fvid_media::owned_aac::{
    aac_ld_qmf::{Analysis, Synthesis},
    aac_sbr_qmf::Complex,
};
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-ld-qmf128")
}
fn f64s(name: &str) -> Vec<f64> {
    std::fs::read(root().join(name))
        .unwrap()
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn f32s(name: &str) -> Vec<f64> {
    std::fs::read(root().join(name))
        .unwrap()
        .chunks_exact(4)
        .map(|b| f64::from(f32::from_le_bytes(b.try_into().unwrap())))
        .collect()
}
fn error(got: &[f64], want: &[f64]) -> f64 {
    assert_eq!(got.len(), want.len());
    got.iter()
        .zip(want)
        .map(|(a, b)| (a - b).abs())
        .fold(0., f64::max)
}
#[test]
fn wide_ld_banks_match_scalar_oracle_original_and_precision_control() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let mut maximum = 0f64;
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let pcm = f32s(&format!("{name}.input.f32le"));
        let mut a = Analysis::<128>::new().unwrap();
        let rows = a.process(&pcm).unwrap();
        let flat: Vec<_> = rows
            .iter()
            .flat_map(|r| r.iter().flat_map(|z| [z.re, z.im]))
            .collect();
        let scalar = f64s(&format!("{name}.analysis.f64le"));
        let e = error(&flat, &scalar);
        maximum = maximum.max(e);
        assert!(e < 1e-11, "{name} analysis scalar {e}");
        // Original ISO uses float PI; the precision control and independently
        // audited float-stage model isolate that phase/rounding difference.
        let e = error(&flat, &f32s(&format!("{name}.original.analysis.f32le")));
        assert!(e < 1.1e-4, "{name} original analysis {e}");
        let e = error(&flat, &f32s(&format!("{name}.precise.analysis.f32le")));
        assert!(e < 5e-6, "{name} precision analysis {e}");
        let mut s = Synthesis::<128>::new().unwrap();
        let actual = s.process(&rows).unwrap();
        let e = error(&actual, &f64s(&format!("{name}.output.f64le")));
        maximum = maximum.max(e);
        assert!(e < 1e-11, "{name} synthesis scalar {e}");
        let e = error(&actual, &f64s(&format!("{name}.original.output.f64le")));
        assert!(e < 3e-6, "{name} original roundtrip {e}");
        let mut split = Analysis::<128>::new().unwrap();
        let mut chunks = split.process(&pcm[..384]).unwrap();
        let checkpoint = split.clone();
        let mut bad = vec![0.; 128];
        bad[37] = f64::NAN;
        assert!(split.process(&bad).is_err());
        assert_eq!(split, checkpoint);
        chunks.extend(split.process(&pcm[384..]).unwrap());
        assert_eq!(chunks, rows);
        split.reset();
        assert_eq!(split.process(&pcm).unwrap(), rows);
        let mut ss = Synthesis::<128>::new().unwrap();
        let mut out = ss.process(&rows[..7]).unwrap();
        let checkpoint = ss.clone();
        let mut bad = vec![[Complex::default(); 128]];
        bad[0][127].im = f64::INFINITY;
        assert!(ss.process(&bad).is_err());
        assert_eq!(ss, checkpoint);
        out.extend(ss.process(&rows[7..]).unwrap());
        assert_eq!(out, actual);
        ss.reset();
        assert_eq!(ss.process(&rows).unwrap(), actual);
    }
    for c in m["synthesis_cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let input = f32s(&format!("{name}.complex.f32le"));
        let rows: Vec<[Complex; 128]> = input
            .chunks_exact(256)
            .map(|r| {
                std::array::from_fn(|i| Complex {
                    re: r[2 * i],
                    im: r[2 * i + 1],
                })
            })
            .collect();
        let mut s = Synthesis::<128>::new().unwrap();
        let actual = s.process(&rows).unwrap();
        let e = error(&actual, &f64s(&format!("{name}.output.f64le")));
        maximum = maximum.max(e);
        assert!(e < 1e-11, "{name} synthesis {e}");
        assert!(actual.iter().any(|v| v.abs() > 1e-8));
        let e = error(&actual, &f64s(&format!("{name}.original.output.f64le")));
        assert!(e < 3e-6, "{name} original synthesis {e}");
        let e = error(&actual, &f64s(&format!("{name}.precise.output.f64le")));
        assert!(e < 2e-8, "{name} precision synthesis {e}");
    }
    eprintln!("128-bin LD bank scalar maximum difference {maximum}");
}
