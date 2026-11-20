use fvid_media::owned_aac::{
    aac_ld_mps_config::Config,
    aac_ld_mps_entropy::Kind,
    aac_ld_mps_frame::ParameterId,
    aac_ld_mps_matrix_time::{Layout, MatrixTime},
    aac_ld_mps_prepare::{self as physical, Decoder, Shape, SmoothControl, SmoothState},
    bits::BitReader,
};
use serde_json::{Value, json};
use std::path::PathBuf;
fn root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn json_file(name: &str, file: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root(name).join(file)).unwrap()).unwrap()
}
fn id(p: &ParameterId) -> String {
    match p {
        ParameterId::OttCld(i) => format!("ott-cld:{i}"),
        ParameterId::OttIcc(Some(i)) => format!("ott-icc:{i}"),
        ParameterId::OttIcc(None) => "ott-icc:all".into(),
        ParameterId::TttCpc {
            box_index,
            high,
            component,
        } => format!("ttt-cpc:{box_index}:{}:{component}", u8::from(*high)),
        ParameterId::TttCld {
            box_index,
            high,
            component,
        } => format!("ttt-cld:{box_index}:{}:{component}", u8::from(*high)),
        ParameterId::TttIcc { box_index, high } => {
            format!("ttt-icc:{box_index}:{}", u8::from(*high))
        }
        ParameterId::DownmixGain(i) => format!("downmix:{i}"),
        ParameterId::ArbitraryCld {
            extension,
            box_index,
        } => format!("arb:{extension}:{box_index}"),
    }
}
fn close(a: &Value, b: &Value) {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            assert!((a - b).abs() <= 2e-12 * (1.0 + b.abs()), "{a} != {b}");
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                close(a, b);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len());
            for (k, a) in a {
                close(a, &b[k]);
            }
        }
        _ => assert_eq!(a, b),
    }
}
fn wire(blob: &[u8], f: &Value, at: usize) -> Vec<u8> {
    let n = f["bits"].as_u64().unwrap() as usize;
    let start = f["offset"].as_u64().unwrap() as usize;
    let mut out = vec![0; (at + n + 8).div_ceil(8)];
    for i in 0..n {
        out[(at + i) / 8] |= ((blob[start + i / 8] >> (7 - i % 8)) & 1) << (7 - (at + i) % 8);
    }
    for i in 0..8 {
        out[(at + n + i) / 8] |= ((0x5au8 >> (7 - i)) & 1) << (7 - (at + n + i) % 8);
    }
    out
}
fn cfg(c: &Value, blob: &[u8]) -> Config {
    let at = c["config_offset"].as_u64().unwrap() as usize;
    Config::parse(&blob[at..at + c["config_bytes"].as_u64().unwrap() as usize]).unwrap()
}
fn metadata(p: &physical::Prepared) -> Value {
    json!({"slots":p.slots,"parameters":p.parameters.iter().map(|p|json!({"id":id(&p.id),"kind":match p.kind{Kind::Cld=>"CLD",Kind::Icc=>"ICC",Kind::Cpc=>"CPC"},"start":p.start_band,"indices":p.indices,"values":p.values})).collect::<Vec<_>>(),"smoothing":p.smoothing.iter().map(|s|json!({"time":s.time,"mask":s.mask})).collect::<Vec<_>>(),"temporal":match&p.temporal_shape{Shape::Disabled=>json!({"kind":"disabled"}),Shape::Stp(on)=>json!({"kind":"stp","enabled":on}),Shape::Ges(ch)=>json!({"kind":"ges","envelopes":ch})}})
}
#[test]
fn physical_preparation_couples_entropy_history_energy_and_smoothing_at_all_offsets() {
    let oracle = json_file("aac-ld-mps-prepare", "manifest.json");
    let m = json_file("aac-ld-mps-frame", "manifest.json");
    let configs = std::fs::read(root("aac-ld-mps-frame").join("configs.bin")).unwrap();
    let packets = std::fs::read(root("aac-ld-mps-frame").join("packets.bin")).unwrap();
    let mut accepted = 0;
    let mut refused = 0;
    for (c, o) in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        assert_eq!(c["name"], o["name"]);
        let config = cfg(c, &configs);
        if !o["supported"].as_bool().unwrap() {
            let e = Decoder::new(&config).unwrap_err().to_string();
            assert!(e.contains("energy quantization for this tree/TTT/arbitrary-tree configuration is not yet qualified"));
            refused += 1;
            continue;
        }
        accepted += 1;
        for at in 0..8 {
            let initial = Decoder::new(&config).unwrap();
            let mut decoder = initial.clone();
            for (f, expected) in c["frames"]
                .as_array()
                .unwrap()
                .iter()
                .zip(o["frames"].as_array().unwrap())
            {
                let saved = decoder.clone();
                let bytes = wire(&packets, f, at);
                let n = f["bits"].as_u64().unwrap() as usize;
                let mut truncated = BitReader::with_bit_len(&bytes, at + n - 1).unwrap();
                truncated.skip(at).unwrap();
                assert!(decoder.decode(&mut truncated).is_err());
                assert_eq!(decoder, saved);
                assert_eq!(truncated.position(), at);
                let mut bits = BitReader::new(&bytes);
                bits.skip(at).unwrap();
                let p = decoder.decode(&mut bits).unwrap();
                assert_eq!(bits.position(), at + n);
                assert_eq!(bits.read(8).unwrap(), 0x5a);
                let actual = metadata(&p);
                close(&actual, expected);
                let mut replay = saved;
                let mut bits = BitReader::new(&bytes);
                bits.skip(at).unwrap();
                assert_eq!(replay.decode(&mut bits).unwrap(), p);
                assert_eq!(replay, decoder);
            }
            decoder.reset();
            assert_eq!(decoder, initial);
        }
    }
    assert_eq!((accepted, refused), (161, 175));
}
#[test]
fn every_energy_quantizer_point_matches_high_precision_oracle_and_conserves_energy() {
    let oracle = json_file("aac-ld-mps-prepare", "manifest.json");
    assert_eq!(oracle["scalar"].as_array().unwrap().len(), 1116);
    for p in oracle["scalar"].as_array().unwrap() {
        let out = physical::energy_split(
            p["index"].as_i64().unwrap() as i16,
            p["input"].as_f64().unwrap(),
            p["mode"].as_u64().unwrap() as u8,
        )
        .unwrap();
        assert_eq!(i64::from(out.index), p["adjusted"].as_i64().unwrap());
        close(&json!(out.left_db), &p["left"]);
        close(&json!(out.right_db), &p["right"]);
        let parent = p["input"].as_f64().unwrap();
        assert!(
            (10f64.powf((out.left_db - parent) / 10.0)
                + 10f64.powf((out.right_db - parent) / 10.0)
                - 1.0)
                .abs()
                < 2e-13
        );
    }
    for db in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.1] {
        assert!(physical::energy_split(0, db, 1).is_err());
    }
    assert!(physical::energy_split(16, 0.0, 1).is_err());
    assert!(physical::energy_split(0, 0.0, 3).is_err());
}
fn controls(f: &Value) -> Vec<SmoothControl> {
    f["controls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| SmoothControl {
            time: s["time"].as_u64().unwrap() as u16,
            mask: s["mask"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_bool().unwrap())
                .collect(),
        })
        .collect()
}
fn vectors(v: &Value) -> Vec<Vec<f64>> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        })
        .collect()
}
#[test]
fn matrix_time_matches_exact_rational_sequences_with_replay_and_late_error_rollback() {
    let oracle = json_file("aac-ld-mps-prepare", "manifest.json");
    assert_eq!(oracle["matrix"].as_array().unwrap().len(), 28);
    for c in oracle["matrix"].as_array().unwrap() {
        let l = c["layout"].as_array().unwrap();
        let initial = MatrixTime::new(
            c["bands"].as_u64().unwrap() as u8,
            Layout {
                planes: l[0].as_u64().unwrap() as u8,
                rows: l[1].as_u64().unwrap() as u8,
                columns: l[2].as_u64().unwrap() as u8,
            },
        )
        .unwrap();
        let mut engine = initial.clone();
        for f in c["frames"].as_array().unwrap() {
            let saved = engine.clone();
            let slots = f["slots"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect::<Vec<_>>();
            let time = f["frame_slots"].as_u64().unwrap() as u8;
            let cutoff = f["residual_cutoff"].as_u64().unwrap() as u8;
            let ctl = controls(f);
            let raw = vectors(&f["raw"]);
            let mut bad = raw.clone();
            *bad.last_mut().unwrap().last_mut().unwrap() = f64::NAN;
            assert!(engine.process(time, &slots, &ctl, &bad, cutoff).is_err());
            assert_eq!(engine, saved);
            let out = engine.process(time, &slots, &ctl, &raw, cutoff).unwrap();
            close(&json!(out.endpoints), &f["endpoints"]);
            close(&json!(out.slots), &f["output"]);
            let mut replay = saved;
            assert_eq!(
                replay.process(time, &slots, &ctl, &raw, cutoff).unwrap(),
                out
            );
            assert_eq!(replay, engine);
        }
        engine.reset();
        assert_eq!(engine, initial);
    }
}
#[test]
fn numeric_grids_qmf_geometry_extreme_layouts_and_inactive_leading_keep_are_bounded() {
    let facts = json_file("aac-ld-mps-prepare", "facts.json");
    for (kind, name, offset) in [
        (Kind::Cld, "dequantCLD", 15),
        (Kind::Icc, "dequantICC", 0),
        (Kind::Cpc, "dequantCPC", 20),
    ] {
        for (i, v) in facts[name].as_array().unwrap().iter().enumerate() {
            assert_eq!(
                physical::dequantize(kind, i as i16 - offset).unwrap(),
                v.as_f64().unwrap()
            );
        }
        assert!(physical::dequantize(kind, -offset - 1).is_err());
        assert!(
            physical::dequantize(kind, facts[name].as_array().unwrap().len() as i16 - offset)
                .is_err()
        );
    }
    assert_eq!(physical::dequantize(Kind::Icc, 7).unwrap(), -0.99);
    for p in [4, 5, 7, 9, 12, 15, 23] {
        for b in [32, 64, 128] {
            let map = physical::qmf_map(p, b).unwrap();
            let saved = facts[format!("QMF{p}")].as_array().unwrap();
            assert_eq!(
                map,
                saved[..b]
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(
            physical::expansion23(p).unwrap().as_slice(),
            facts[format!("MAP{p}")]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        );
    }
    assert!(physical::qmf_map(6, 64).is_err());
    assert!(physical::qmf_map(23, 63).is_err());
    let mut state = SmoothState::new(23).unwrap();
    let initial = state.clone();
    use fvid_media::owned_aac::aac_ld_mps_frame::Smoothing;
    let keep = Smoothing {
        mode: 1,
        time: None,
        stride: None,
        groups: vec![],
    };
    let controls = state.decode(&[keep.clone()]).unwrap();
    assert_eq!(controls[0].time, 0);
    assert!(controls[0].mask.iter().all(|v| !*v));
    let bad = Smoothing {
        mode: 3,
        time: Some(0),
        stride: Some(5),
        groups: vec![],
    };
    assert!(state.decode(&[keep, bad]).is_err());
    assert_eq!(state, initial);
    let layout = Layout {
        planes: 7,
        rows: 8,
        columns: 13,
    };
    let mut matrices = MatrixTime::new(23, layout).unwrap();
    let before = matrices.clone();
    let size = 23 * 7 * 8 * 13;
    let raw = vec![vec![1.0; size]];
    let out = matrices.process(1, &[0], &controls, &raw, 0).unwrap();
    assert_eq!(out.slots, raw);
    let active = vec![SmoothControl {
        time: 64,
        mask: vec![true; 23],
    }];
    let raw_two = vec![vec![2.0; size]];
    let filtered = matrices.process(32, &[31], &active, &raw_two, 0).unwrap();
    assert_eq!(filtered.endpoints[0], vec![1.5; size]);
    assert_eq!(filtered.slots[0], vec![65.0 / 64.0; size]);
    assert_eq!(filtered.slots[31], vec![1.5; size]);
    let bypass = matrices.process(1, &[0], &active, &raw_two, 23).unwrap();
    assert_eq!(bypass.slots, raw_two);
    matrices.reset();
    assert_eq!(matrices, before);
    assert!(MatrixTime::new(24, layout).is_err());
    assert!(
        MatrixTime::new(
            23,
            Layout {
                planes: 8,
                ..layout
            }
        )
        .is_err()
    );
    assert!(MatrixTime::new(23, Layout { rows: 9, ..layout }).is_err());
    assert!(
        MatrixTime::new(
            23,
            Layout {
                columns: 14,
                ..layout
            }
        )
        .is_err()
    );
}

#[test]
fn physical_rate_thresholds_and_all_fixed_gain_indices_match_saved_controls() {
    let oracle = json_file("aac-ld-mps-prepare", "manifest.json");
    let m = json_file("aac-ld-mps-frame", "manifest.json");
    let configs = std::fs::read(root("aac-ld-mps-frame").join("configs.bin")).unwrap();
    let packets = std::fs::read(root("aac-ld-mps-frame").join("packets.bin")).unwrap();
    let c = &m["cases"][0];
    let baseline = cfg(c, &configs);
    let f = &c["frames"][0];
    let bytes = wire(&packets, f, 0);
    assert_eq!(oracle["rates_gains"].as_array().unwrap().len(), 1400);
    for r in oracle["rates_gains"].as_array().unwrap() {
        let mut config = baseline.clone();
        config.sample_rate = r["rate"].as_u64().unwrap() as u32;
        config.surround_gain = Some(r["surround"].as_u64().unwrap() as u8);
        config.lfe_gain = Some(r["lfe"].as_u64().unwrap() as u8);
        config.downmix_gain = r["downmix"].as_u64().unwrap() as u8;
        let out = Decoder::new(&config)
            .unwrap()
            .decode(&mut BitReader::new(&bytes))
            .unwrap();
        assert_eq!(out.qmf_bands as u64, r["qmf_bands"].as_u64().unwrap());
        close(
            &json!([out.surround_gain, out.lfe_gain, out.clip_protection_gain]),
            &r["gains"],
        );
        assert_eq!(
            out.qmf_parameter_bands,
            physical::qmf_map(config.parameter_bands, out.qmf_bands).unwrap()
        );
    }
    let mut config = baseline.clone();
    config.sample_rate = 0;
    assert!(Decoder::new(&config).is_err());
    config = baseline.clone();
    config.downmix_gain = 8;
    assert!(Decoder::new(&config).is_err());
    config = baseline.clone();
    config.lfe_gain = Some(5);
    assert!(Decoder::new(&config).is_err());
    config = baseline;
    config.surround_gain = Some(5);
    assert!(Decoder::new(&config).is_err());
}

#[test]
fn active_arbitrary_trees_at_quant_zero_have_physical_acceptance_for_every_tree() {
    let oracle = json_file("aac-ld-mps-prepare", "manifest.json");
    let m = json_file("aac-ld-mps-frame", "manifest.json");
    let configs = std::fs::read(root("aac-ld-mps-frame").join("configs.bin")).unwrap();
    let packets = std::fs::read(root("aac-ld-mps-frame").join("packets.bin")).unwrap();
    assert_eq!(oracle["extra_cases"].as_array().unwrap().len(), 56);
    for o in oracle["extra_cases"].as_array().unwrap() {
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == o["base_case"])
            .unwrap();
        let mut config = cfg(c, &configs);
        config.quantization_mode = 0;
        for at in 0..8 {
            let initial = Decoder::new(&config).unwrap();
            let mut decoder = initial.clone();
            for (f, expected) in c["frames"]
                .as_array()
                .unwrap()
                .iter()
                .zip(o["frames"].as_array().unwrap())
            {
                let bytes = wire(&packets, f, at);
                let saved = decoder.clone();
                let mut bits = BitReader::new(&bytes);
                bits.skip(at).unwrap();
                let p = decoder.decode(&mut bits).unwrap();
                close(&metadata(&p), expected);
                assert_eq!(bits.position(), at + f["bits"].as_u64().unwrap() as usize);
                assert_eq!(bits.read(8).unwrap(), 0x5a);
                let mut replay = saved;
                let mut bits = BitReader::new(&bytes);
                bits.skip(at).unwrap();
                assert_eq!(replay.decode(&mut bits).unwrap(), p);
                assert_eq!(replay, decoder);
            }
            decoder.reset();
            assert_eq!(decoder, initial);
        }
    }
}
