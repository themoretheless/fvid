use fvid_media::owned_aac::{
    aac_ld_mps_config::Config,
    aac_ld_mps_entropy::Kind,
    aac_ld_mps_frame::{Decoder, Frame, ParameterId, ParameterRows, TemporalShape},
    bits::BitReader,
};
use serde_json::{Value, json};
use std::path::PathBuf;
fn corpus() -> (Value, Vec<u8>, Vec<u8>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-frame");
    (
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap(),
        std::fs::read(root.join("configs.bin")).unwrap(),
        std::fs::read(root.join("packets.bin")).unwrap(),
    )
}
fn config(c: &Value, blob: &[u8]) -> Config {
    let start = c["config_offset"].as_u64().unwrap() as usize;
    Config::parse(&blob[start..start + c["config_bytes"].as_u64().unwrap() as usize]).unwrap()
}
fn id(id: &ParameterId) -> String {
    match id {
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
fn parameter(p: &ParameterRows) -> Value {
    json!({"id":id(&p.id),"kind":match p.kind{Kind::Cld=>"CLD",Kind::Icc=>"ICC",Kind::Cpc=>"CPC"},"start":p.start_band,"values":p.values})
}
fn metadata(f: &Frame) -> Value {
    json!({"slots":f.slots,"independent":f.independent,"parameters":f.parameters.iter().map(parameter).collect::<Vec<_>>(),"smoothing":f.smoothing.iter().map(|s|json!({"mode":s.mode,"time":s.time,"stride":s.stride,"groups":s.groups})).collect::<Vec<_>>(),"temporal":match &f.temporal_shape{TemporalShape::Disabled=>json!({"kind":"disabled"}),TemporalShape::Stp(on)=>json!({"kind":"stp","enabled":on}),TemporalShape::Ges(ch)=>json!({"kind":"ges","envelopes":ch})},"extensions":f.extensions.iter().map(|e|json!({"kind":e.kind,"payload":e.payload.iter().map(|b|format!("{b:02x}")).collect::<String>(),"parameters":e.parameters.iter().map(parameter).collect::<Vec<_>>()})).collect::<Vec<_>>()})
}
fn expected(f: &Value) -> Value {
    let mut e = f.clone();
    e.as_object_mut().unwrap().retain(|k, _| {
        [
            "slots",
            "independent",
            "parameters",
            "smoothing",
            "temporal",
            "extensions",
        ]
        .contains(&k.as_str())
    });
    e
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
fn set(bytes: &mut [u8], at: usize, width: usize, value: usize) {
    for i in 0..width {
        let bit = at + i;
        bytes[bit / 8] = (bytes[bit / 8] & !(1 << (7 - bit % 8)))
            | ((((value >> (width - i - 1)) & 1) as u8) << (7 - bit % 8));
    }
}
#[test]
fn all_tree_frames_match_saved_indices_tools_and_bounded_extensions_at_all_offsets() {
    let (manifest, configs, packets) = corpus();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 336);
    for c in manifest["cases"].as_array().unwrap() {
        for at in 0..8 {
            let initial = Decoder::new(&config(c, &configs)).unwrap();
            let mut decoder = initial.clone();
            for f in c["frames"].as_array().unwrap() {
                let saved = decoder.clone();
                let bytes = wire(&packets, f, at);
                let mut b = BitReader::new(&bytes);
                b.skip(at).unwrap();
                let output = decoder
                    .decode(&mut b)
                    .unwrap_or_else(|e| panic!("{} offset {at} {:?}: {e}", c["name"], f["slots"]));
                assert_eq!(metadata(&output), expected(f), "{} offset {at}", c["name"]);
                assert_eq!(b.position(), at + f["bits"].as_u64().unwrap() as usize);
                assert_eq!(b.read(8).unwrap(), 0x5a);
                let mut replay = saved;
                let mut b = BitReader::new(&bytes);
                b.skip(at).unwrap();
                assert_eq!(replay.decode(&mut b).unwrap(), output);
                assert_eq!(replay, decoder);
            }
            decoder.reset();
            assert_eq!(decoder, initial);
        }
    }
}
#[test]
fn every_frame_prefix_rolls_back_all_tree_and_extension_histories() {
    let (manifest, configs, packets) = corpus();
    let mut checked = 0;
    for c in manifest["cases"].as_array().unwrap() {
        let mut decoder = Decoder::new(&config(c, &configs)).unwrap();
        for f in c["frames"].as_array().unwrap() {
            let n = f["bits"].as_u64().unwrap() as usize;
            for at in [0, 3, 7] {
                let bytes = wire(&packets, f, at);
                for prefix in 0..n {
                    let mut candidate = decoder.clone();
                    let mut b = BitReader::with_bit_len(&bytes, at + prefix).unwrap();
                    b.skip(at).unwrap();
                    assert!(
                        candidate.decode(&mut b).is_err(),
                        "{} prefix {prefix}/{n}",
                        c["name"]
                    );
                    assert_eq!(candidate, decoder);
                    assert_eq!(b.position(), at);
                    checked += 1;
                }
            }
            let bytes = wire(&packets, f, 0);
            decoder.decode(&mut BitReader::new(&bytes)).unwrap();
        }
    }
    eprintln!("{checked} spatial-frame truncated prefixes checked");
    assert_eq!(checked, 777_384);
}
#[test]
fn malformed_slots_predictive_defaults_and_short_extension_children_fail_specifically() {
    let (manifest, configs, packets) = corpus();
    let mut checks = [0; 3];
    for c in manifest["cases"].as_array().unwrap() {
        let initial = Decoder::new(&config(c, &configs)).unwrap();
        let mut decoder = initial.clone();
        for (fi, f) in c["frames"].as_array().unwrap().iter().enumerate() {
            let original = wire(&packets, f, 0);
            let assert_error = |bytes: Vec<u8>, reason: &str| {
                let mut candidate = decoder.clone();
                let mut b = BitReader::new(&bytes);
                let error = candidate.decode(&mut b).unwrap_err().to_string();
                assert!(
                    error.contains(reason),
                    "{}: {error}, wanted {reason}",
                    c["name"]
                );
                assert_eq!(candidate, decoder);
                assert_eq!(b.position(), 0);
            };
            if fi == 1 && f["slots"].as_array().unwrap().len() == 2 {
                let width =
                    (u64::BITS - (c["frame_slots"].as_u64().unwrap() - 1).leading_zeros()) as usize;
                let mut bytes = original.clone();
                set(&mut bytes, 2 + width, width, 0);
                assert_error(bytes, "invalid LD-MPS frame parameter slots");
                checks[0] += 1;
            }
            for field in f["mode_fields"].as_array().unwrap() {
                if field["predictive"] == true {
                    let mut bytes = original.clone();
                    set(
                        &mut bytes,
                        field["position"].as_u64().unwrap() as usize,
                        2,
                        0,
                    );
                    assert_error(bytes, "TTT ICC requires non-default");
                    checks[1] += 1;
                }
            }
            for field in f["extension_lengths"].as_array().unwrap() {
                if field["kind"] == 2 && field["length"].as_u64().unwrap() > 0 {
                    let mut bytes = original.clone();
                    set(
                        &mut bytes,
                        field["position"].as_u64().unwrap() as usize,
                        8,
                        field["length"].as_u64().unwrap() as usize - 1,
                    );
                    assert_error(bytes, "truncated");
                    checks[2] += 1;
                }
            }
            decoder.decode(&mut BitReader::new(&original)).unwrap();
        }
    }
    assert!(checks.iter().all(|&n| n > 0));
}

#[test]
fn saved_tool_failures_and_invalid_empty_coding_preserve_the_whole_frame() {
    let (manifest, configs, packets) = corpus();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-frame");
    let errors = std::fs::read(root.join("errors.bin")).unwrap();
    assert_eq!(manifest["errors"].as_array().unwrap().len(), 5);
    for e in manifest["errors"].as_array().unwrap() {
        let c = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == e["case"])
            .unwrap();
        let cfg = if let Some(hex) = e["config"].as_str() {
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect::<Vec<_>>();
            Config::parse(&bytes).unwrap()
        } else {
            config(c, &configs)
        };
        let mut decoder = Decoder::new(&cfg).unwrap();
        for f in c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .take(e["seed"].as_u64().unwrap() as usize)
        {
            let bytes = wire(&packets, f, 0);
            decoder.decode(&mut BitReader::new(&bytes)).unwrap();
        }
        for at in 0..8 {
            let bytes = wire(&errors, e, at);
            let mut b =
                BitReader::with_bit_len(&bytes, at + e["bits"].as_u64().unwrap() as usize).unwrap();
            b.skip(at).unwrap();
            let mut candidate = decoder.clone();
            let error = candidate.decode(&mut b).unwrap_err().to_string();
            assert!(
                error.contains(e["reason"].as_str().unwrap()),
                "{}: {error}",
                e["name"]
            );
            assert_eq!(candidate, decoder);
            assert_eq!(b.position(), at);
        }
    }
}
