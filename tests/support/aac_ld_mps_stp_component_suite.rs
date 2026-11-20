#[test]
fn independent_adapted_iso_stp_component_matches_all_trees_and_preserves_state() {
    let m = manifest("component.json");
    let inputs = std::fs::read(root().join("component-input.f32le")).unwrap();
    let expected = std::fs::read(root().join("component-expected.f32le")).unwrap();
    let mut count = 0;
    let mut maximum = 0f64;
    let mut changed = 0;
    let signal = |at: &mut usize, slots: usize, channels: usize, bands: usize| {
        (0..slots)
            .map(|_| {
                (0..channels)
                    .map(|_| {
                        (0..bands)
                            .map(|_| {
                                let re = f64::from(f32::from_le_bytes(
                                    inputs[*at..*at + 4].try_into().unwrap(),
                                ));
                                let im = f64::from(f32::from_le_bytes(
                                    inputs[*at + 4..*at + 8].try_into().unwrap(),
                                ));
                                *at += 8;
                                Complex { re, im }
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    for c in m["cases"].as_array().unwrap() {
        let tree = n(&c["tree"]) as u8;
        let bands = n(&c["bands"]);
        let channels = n(&c["outputs"]);
        let mut decoder = Decoder::new(tree, bands).unwrap();
        let fresh = decoder.clone();
        for block in c["blocks"].as_array().unwrap() {
            let slots = n(&block["slots"]);
            let mut at = n(&block["offset"]);
            let mut dry = signal(&mut at, slots, channels, bands);
            let mut wet = signal(&mut at, slots, channels, bands);
            let original_dry = dry.clone();
            let original_wet = wet.clone();
            let flags = block["flags"]
                .as_array()
                .map(|r| r.iter().map(|b| b.as_bool().unwrap()).collect::<Vec<_>>());
            let before = decoder.clone();
            let mut chunked = decoder.clone();
            let mut chunk_dry = Vec::new();
            let mut chunk_wet = Vec::new();
            for t in 0..slots {
                let mut d = vec![dry[t].clone()];
                let mut w = vec![wet[t].clone()];
                chunked.apply(&mut d, &mut w, flags.as_deref()).unwrap();
                chunk_dry.extend(d);
                chunk_wet.extend(w);
            }
            decoder.apply(&mut dry, &mut wet, flags.as_deref()).unwrap();
            assert_eq!(decoder, chunked);
            assert_eq!(dry, chunk_dry);
            assert_eq!(wet, chunk_wet);
            let mut at = n(&block["expected_offset"]);
            for t in 0..slots {
                for ch in 0..channels {
                    for b in 0..bands {
                        let got = Complex {
                            re: dry[t][ch][b].re + wet[t][ch][b].re,
                            im: dry[t][ch][b].im + wet[t][ch][b].im,
                        };
                        for actual in [got.re, got.im] {
                            let want = f64::from(f32::from_le_bytes(
                                expected[at..at + 4].try_into().unwrap(),
                            ));
                            at += 4;
                            maximum = maximum.max((actual - want).abs());
                            count += 1;
                            assert!(
                                (actual - want).abs() < 2e-7,
                                "tree{tree} B{bands} t{t} ch{ch} b{b}: {actual} vs {want}"
                            );
                        }
                        let plain = Complex {
                            re: original_dry[t][ch][b].re + original_wet[t][ch][b].re,
                            im: original_dry[t][ch][b].im + original_wet[t][ch][b].im,
                        };
                        if flags.is_none() || b < 12 {
                            assert_eq!(got, plain);
                        }
                        if b < 12 {
                            assert_eq!(wet[t][ch][b], Complex::default());
                        }
                        if (got.re - plain.re).abs() > 1e-8 {
                            changed += 1;
                        }
                    }
                }
            }
            let mut retry = before.clone();
            let mut d = original_dry.clone();
            let mut bad = original_wet.clone();
            bad[slots - 1][channels - 1][bands - 1].re = f64::NAN;
            let retained = d.clone();
            assert!(retry.apply(&mut d, &mut bad, flags.as_deref()).is_err());
            assert_eq!(retry, before);
            assert_eq!(d, retained);
            // Force a late overflow in bin0, outside the energy summation.
            // All slots have advanced history by the final finite-output check.
            let mut overflow_dry = original_dry.clone();
            let mut overflow_wet = original_wet.clone();
            overflow_dry[slots - 1][0][0].re = f64::MAX;
            overflow_wet[slots - 1][0][0].re = f64::MAX;
            let retained_dry = overflow_dry.clone();
            let retained_wet = overflow_wet.clone();
            assert!(
                retry
                    .apply(&mut overflow_dry, &mut overflow_wet, flags.as_deref())
                    .is_err()
            );
            assert_eq!(retry, before);
            assert_eq!(overflow_dry, retained_dry);
            assert_eq!(overflow_wet, retained_wet);
            let mut d = original_dry;
            let mut w = original_wet;
            retry.apply(&mut d, &mut w, flags.as_deref()).unwrap();
            assert_eq!(retry, decoder);
            assert_eq!(d, dry);
            assert_eq!(w, wet);
        }
        decoder.reset();
        assert_eq!(decoder, fresh);
        assert!(decoder.retained_payload_bytes().unwrap() < 1024);
    }
    assert_eq!(count, EXPECTED_SCALARS);
    assert!(changed > 10_000);
    eprintln!(
        "STP independent component scalars {count}, maximum difference {maximum}, changed {changed}"
    );
}
