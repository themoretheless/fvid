//! Component/overlap acceptance; this does not enable native residual playback.
use fvid_media::owned_aac::{
    aac_ld_mps_residual::{Block, Spectrum},
    aac_mps_mdct_qmf::Decoder,
    aac_sbr_qmf::Complex,
    aac_synthesis::{WindowSequence as W, WindowShape},
};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-mps-mdct-qmf")
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn spectra(input: &[u8], case: &Value) -> Vec<Block> {
    let sequence = [W::OnlyLong, W::LongStart, W::EightShort, W::LongStop][n(&case["sequence"])];
    let spectrum = |v: &Value| {
        let at = n(&v["offset"]);
        Spectrum {
            sequence,
            shape: WindowShape::Sine,
            coefficients: input[at..at + 4096]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect(),
        }
    };
    case["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| Block {
            first: spectrum(&p["first"]),
            extra: if p["extra"].is_null() {
                None
            } else {
                Some(spectrum(&p["extra"]))
            },
        })
        .collect()
}
#[test]
fn residual_conversion_matches_unmodified_iso_and_keeps_overlap_checkpoint_reset() {
    let m: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let input = std::fs::read(root().join("input.f32le")).unwrap();
    let expected = std::fs::read(root().join("expected.f32le")).unwrap();
    let mut maximum = 0f64;
    let mut count = 0;
    let mut overlap = 0;
    let mut identity_gap = 0f64;
    for c in m["cases"].as_array().unwrap() {
        let slots = n(&c["update"]) * n(&c["frames"]);
        let mut decoder = Decoder::new(slots, n(&c["frames"])).unwrap();
        let fresh = decoder.clone();
        for (index, frame) in c["blocks"].as_array().unwrap().iter().enumerate() {
            let blocks = spectra(&input, frame);
            let before = decoder.clone();
            let mut replay = decoder.clone();
            let out = decoder.process(&blocks).unwrap();
            assert_eq!(replay.process(&blocks).unwrap(), out);
            assert_eq!(replay, decoder);
            assert_eq!(out.len(), slots);
            let mut at = n(&frame["expected_offset"]);
            for row in &out {
                assert_eq!(row.len(), 64);
                for z in row {
                    for got in [z.re, z.im] {
                        let want =
                            f64::from(f32::from_le_bytes(expected[at..at + 4].try_into().unwrap()));
                        at += 4;
                        maximum = maximum.max((got - want).abs());
                        identity_gap = identity_gap.max(want.abs());
                        count += 1;
                        assert!(
                            (got - want).abs() < 5e-7,
                            "update{} frames{} mode{} frame{index}: {got} vs {want}",
                            c["update"],
                            c["frames"],
                            c["mode"]
                        );
                    }
                }
            }
            if c["mode"] == "silent" {
                assert!(out.iter().flatten().all(|v| *v == Complex::default()));
            }
            if c["mode"] != "silent"
                && index == 6
                && out
                    .iter()
                    .flatten()
                    .any(|z| z.re.abs() + z.im.abs() > 1e-10)
            {
                overlap += 1;
            }
            let mut bad = blocks.clone();
            bad.last_mut().unwrap().first.coefficients[1023] = f32::NAN;
            let mut retry = before.clone();
            assert!(retry.process(&bad).is_err());
            assert_eq!(retry, before);
            let mut short = blocks.clone();
            short.last_mut().unwrap().first.coefficients.pop();
            assert!(retry.process(&short).is_err());
            assert_eq!(retry, before);
            assert_eq!(retry.process(&blocks).unwrap(), out);
            assert_eq!(retry, decoder);
        }
        decoder.reset();
        assert_eq!(decoder, fresh);
        assert!(decoder.retained_payload_bytes().unwrap() >= slots * 64 * 16);
        let frame = &c["blocks"][0];
        let blocks = spectra(&input, frame);
        let mut fresh2 = fresh.clone();
        assert_eq!(
            decoder.process(&blocks).unwrap(),
            fresh2.process(&blocks).unwrap()
        );
    }
    assert_eq!(m["cases"].as_array().unwrap().len(), 24);
    assert!(overlap >= 8);
    assert!(identity_gap > 0.01);
    eprintln!(
        "Residual MDCT-QMF scalars {count}, maximum difference {maximum}, zero-output gap {identity_gap}, carried silent frames {overlap}"
    );
    assert!(Decoder::new(16, 3).is_err());
    assert!(
        Decoder::new(8, 1)
            .unwrap_err()
            .to_string()
            .contains("unqualified")
    );
}
