use fvid_media::owned_aac::{aac_ld_mps_ges::Decoder, aac_sbr_qmf::Complex};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-mps128-ges")
}
fn manifest(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn targets(b: &Value) -> Option<Vec<Option<Vec<f64>>>> {
    b["targets"].as_array().map(|rows| {
        rows.iter()
            .map(|row| {
                row.as_array()
                    .map(|r| r.iter().map(|v| v.as_f64().unwrap()).collect())
            })
            .collect()
    })
}
fn signal(
    raw: &[u8],
    at: &mut usize,
    slots: usize,
    channels: usize,
    bands: usize,
) -> Vec<Vec<Vec<Complex>>> {
    (0..slots)
        .map(|_| {
            (0..channels)
                .map(|_| {
                    (0..bands)
                        .map(|_| {
                            let re = f64::from(f32::from_le_bytes(
                                raw[*at..*at + 4].try_into().unwrap(),
                            ));
                            let im = f64::from(f32::from_le_bytes(
                                raw[*at + 4..*at + 8].try_into().unwrap(),
                            ));
                            *at += 8;
                            Complex { re, im }
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}
#[test]
fn guided_envelopes_match_unmodified_iso_reference_and_keep_low_bins_and_checkpoints() {
    let m = manifest("component.json");
    let oracle = manifest("component-oracle.json");
    let input = std::fs::read(root().join("component-input.f32le")).unwrap();
    let expected = std::fs::read(root().join("component-expected.f32le")).unwrap();
    let mut maximum = 0f64;
    let mut scalars = 0;
    let mut upper_effects = 0;
    for (c, o) in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        let bands = n(&c["bands"]);
        let inputs = n(&c["inputs"]);
        let outputs = n(&c["outputs"]);
        let fresh = Decoder::new(n(&c["tree"]) as u8, bands).unwrap();
        let mut decoder = fresh.clone();
        let retained = decoder.retained_payload_bytes().unwrap();
        assert_eq!(retained, (inputs.min(2) + outputs) * 11 * 8);
        for (b, reference) in c["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .zip(o["blocks"].as_array().unwrap())
        {
            let mut at = n(&b["offset"]);
            let slots = n(&b["slots"]);
            let downmix = signal(&input, &mut at, slots, inputs, bands);
            let mut dry = signal(&input, &mut at, slots, outputs, bands);
            let wet = signal(&input, &mut at, slots, outputs, bands);
            let before = dry.clone();
            let saved = decoder.clone();
            let envelope = targets(b);
            decoder
                .apply(&downmix, &mut dry, &wet, envelope.as_deref())
                .unwrap();
            let mut at = n(&reference["expected_offset"]);
            let want = signal(&expected, &mut at, slots, outputs, bands);
            for t in 0..slots {
                for ch in 0..outputs {
                    for band in 0..bands {
                        let a = dry[t][ch][band];
                        let e = want[t][ch][band];
                        let delta = (a.re - e.re).abs().max((a.im - e.im).abs());
                        maximum = maximum.max(delta);
                        scalars += 2;
                        if band == 127
                            && (a.re - before[t][ch][band].re)
                                .abs()
                                .max((a.im - before[t][ch][band].im).abs())
                                > 1e-8
                        {
                            upper_effects += 1;
                        }
                        assert!(
                            delta < 2e-7,
                            "{} tree{} bands{bands} band{band}: {a:?} != {e:?}",
                            c["mode"],
                            c["tree"]
                        );
                        if band < 6 || envelope.is_none() {
                            assert_eq!(a.re.to_bits(), before[t][ch][band].re.to_bits());
                            assert_eq!(a.im.to_bits(), before[t][ch][band].im.to_bits());
                        }
                    }
                }
            }
            let mut replay = saved.clone();
            let mut again = before.clone();
            replay
                .apply(&downmix, &mut again, &wet, envelope.as_deref())
                .unwrap();
            assert_eq!(again, dry);
            assert_eq!(replay, decoder);
            assert_eq!(decoder.retained_payload_bytes().unwrap(), retained);
            if slots > 1 {
                let split = slots / 2;
                let mut fragmented = saved.clone();
                let part = |a: usize, b: usize| {
                    envelope.as_ref().map(|rows| {
                        rows.iter()
                            .map(|row| row.as_ref().map(|r| r[a..b].to_vec()))
                            .collect::<Vec<_>>()
                    })
                };
                let mut first = before[..split].to_vec();
                let mut last = before[split..].to_vec();
                let e = part(0, split);
                fragmented
                    .apply(&downmix[..split], &mut first, &wet[..split], e.as_deref())
                    .unwrap();
                let e = part(split, slots);
                fragmented
                    .apply(&downmix[split..], &mut last, &wet[split..], e.as_deref())
                    .unwrap();
                first.extend(last);
                assert_eq!(first, dry);
                assert_eq!(fragmented, decoder);
            }
            let mut bad = wet.clone();
            bad[slots - 1][outputs - 1][bands - 1].re = f64::NAN;
            let mut untouched = before.clone();
            let state = decoder.clone();
            assert!(
                decoder
                    .apply(&downmix, &mut untouched, &bad, envelope.as_deref())
                    .is_err()
            );
            assert_eq!(untouched, before);
            assert_eq!(decoder, state);
        }
        decoder.reset();
        assert_eq!(decoder, fresh);
    }
    assert_eq!(scalars, 1_837_056);
    assert!(upper_effects > 100);
    eprintln!("GES independent component scalars {scalars}; maximum difference {maximum}");
}
