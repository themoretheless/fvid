const CORPUS: &str = "aac-ld-mps-arbdmx-switch";
const CONTROLS: usize = 10;
const PROGRAMMES: usize = 330;
include!("support/aac_ld_mps_short_pcm.rs");

#[test]
fn changing_ssc_gain_mode_keeps_the_continuous_compensation_waveform() {
    for case in m()["cases"].as_array().unwrap() {
        let mut actual = Vec::new();
        let mut continuous = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(&case["video"]), &mut actual).unwrap();
        fvid::native_media::decode_mp4_aac_pcm(&read(&case["control"]), &mut continuous).unwrap();
        assert_eq!(actual, continuous, "{}", case["name"]);
    }
}

#[test]
fn failure_after_gain_mode_change_keeps_core_spatial_and_header_history() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let manifest = m();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    assert_eq!(manifest["errors"].as_array().unwrap().len(), 8);
    for bad in manifest["errors"].as_array().unwrap() {
        let case = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == bad["good"])
            .unwrap();
        let mut prefix = Vec::new();
        let error =
            fvid::native_media::decode_mp4_aac_pcm(&read(&bad["video"]), &mut prefix).unwrap_err();
        assert!(error.to_string().contains("truncated"), "{error}");
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(&case["video"]), &mut full).unwrap();
        assert!(prefix.len() < full.len());
        assert_eq!(prefix, full[..prefix.len()]);
        let rate = n(&case["rate"]) as u32;
        let asc = hex(&case["asc"]);
        let mut actual = NativeAacDecoder::new_with_output_rate(&asc, rate).unwrap();
        let mut control = NativeAacDecoder::new_with_output_rate(&asc, rate).unwrap();
        for (i, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let at = n(&row["offset"]);
            let packet = &blob[at..at + n(&row["bytes"])];
            let pts = (i * n(&case["out_n"])) as i64;
            let duration = n(&case["out_n"]) as u64;
            if i == n(&bad["bad_index"]) {
                let row = &bad["frames"][i];
                let at = n(&row["offset"]);
                let error = actual
                    .decode_timed_in_clock(&blob[at..at + n(&row["bytes"])], pts, duration, rate)
                    .unwrap_err();
                assert!(error.to_string().contains("truncated"), "{error}");
            }
            assert_eq!(
                actual
                    .decode_timed_in_clock(packet, pts, duration, rate)
                    .unwrap(),
                control
                    .decode_timed_in_clock(packet, pts, duration, rate)
                    .unwrap()
            );
        }
        loop {
            let a = actual.finish().unwrap();
            let b = control.finish().unwrap();
            assert_eq!(a, b);
            if a.is_none() {
                break;
            }
        }
    }
}

#[test]
fn switch_control_matrices_match_original_reference_before_temporal_interpolation() {
    use fvid_media::owned_aac::aac_ld_mps_assemble;
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(root().join("oracle.json")).unwrap()).unwrap();
    let golden = std::fs::read(root().join("expected.bin")).unwrap();
    let mut count = 0;
    let mut maximum = 0.0f64;
    let manifest = m();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 132);
    for control in oracle["cases"].as_array().unwrap() {
        let case = cases.iter().find(|c| c["name"] == control["name"]).unwrap();
        assert_eq!(case["name"], control["name"]);
        let cfg = Config::parse(&hex(&case["ssc"])).unwrap();
        assert!(cfg.arbitrary_downmix);
        let mut decoder = aac_ld_mps_assemble::Decoder::new(&cfg).unwrap();
        for (wire, expected) in case["reference"]
            .as_array()
            .unwrap()
            .iter()
            .zip(control["frames"].as_array().unwrap())
        {
            let packet = hex(wire);
            let saved = decoder.clone();
            let mut short = BitReader::new(&packet[..packet.len() - 3]);
            assert!(decoder.decode(&mut short).is_err());
            assert_eq!(short.position(), 0);
            assert_eq!(decoder, saved);
            let actual = decoder.decode(&mut BitReader::new(&packet)).unwrap();
            for (i, matrix) in [&actual.m1, &actual.dry, &actual.wet]
                .into_iter()
                .enumerate()
            {
                let mut at = n(&expected["offsets"][i]);
                assert_eq!(
                    matrix.endpoints.len(),
                    expected["slots"].as_array().unwrap().len()
                );
                for endpoint in &matrix.endpoints {
                    assert_eq!(endpoint.len(), n(&expected["widths"][i]));
                    for &value in endpoint {
                        let want =
                            f64::from(f32::from_le_bytes(golden[at..at + 4].try_into().unwrap()));
                        assert!(value.is_finite());
                        let error = (value - want).abs();
                        assert!(error < 1e-5, "{} plane{i}: {value} != {want}", case["name"]);
                        maximum = maximum.max(error);
                        count += 1;
                        at += 4;
                    }
                }
            }
        }
    }
    assert_eq!(count * 4, golden.len());
    eprintln!(
        "Gain-mode-switch control C/Decimal oracle: {count} matrix endpoints;max error {maximum}"
    );
}

#[test]
fn clearing_filter_history_at_headers_is_an_observable_negative_control() {
    use std::collections::HashSet;
    let mut checked = HashSet::new();
    let mut maximum = 0.0f64;
    for case in m()["cases"].as_array().unwrap() {
        if !checked.insert((n(&case["tree"]), n(&case["qmf_bins"]))) {
            continue;
        }
        let cfg = Config::parse(&hex(&case["ssc"])).unwrap();
        let mut good = aac_ld_mps_pcm::Decoder::new(&cfg, Mode::Complex).unwrap();
        let mut reset = aac_ld_mps_pcm::Decoder::new(&cfg, Mode::Complex).unwrap();
        let pcm = read(&case["core_gold"]);
        let count = n(&case["out_n"]);
        let channels = n(&case["input_channels"]);
        let boxes = cfg.tree.ott_lfe.len() + usize::from(cfg.tree.ttt_boxes);
        for (i, wire) in case["reference"].as_array().unwrap().iter().enumerate() {
            let planar: Vec<Vec<f64>> = (0..channels)
                .map(|ch| {
                    (0..count)
                        .map(|t| {
                            let at = ((i * count + t) * channels + ch) * 4;
                            f64::from(f32::from_le_bytes(pcm[at..at + 4].try_into().unwrap()))
                        })
                        .collect()
                })
                .collect();
            if i > 0 && case["frames"][i]["header"].as_bool().unwrap() {
                reset.reset();
            }
            let wire = hex(wire);
            let a = good
                .decode(&mut BitReader::new(&wire), &planar, &vec![None; boxes])
                .unwrap();
            let b = reset
                .decode(&mut BitReader::new(&wire), &planar, &vec![None; boxes])
                .unwrap();
            for (a, b) in a.samples.iter().zip(&b.samples) {
                maximum = maximum.max((a - b).abs());
            }
        }
    }
    assert_eq!(checked.len(), 15);
    assert!(
        maximum > 1e-8,
        "forced resets must differ from continuous state"
    );
    eprintln!("Incorrect full-filter reset at SSC headers negative-control PCM gap: {maximum}");
}
