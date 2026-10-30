use fvid_media::owned_aac::{
    aac_ld_mps_assemble::Decoder, aac_ld_mps_config::Config, bits::BitReader,
};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-assemble")
}
#[test]
fn all_ordinary_tree_frames_assemble_transactionally_and_replay_checkpoints() {
    let root = root();
    let m: Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let configs = std::fs::read(root.join("configs.bin")).unwrap();
    let packets = std::fs::read(root.join("packets.bin")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 336);
    let mut frames = 0;
    for c in m["cases"].as_array().unwrap() {
        let co = c["config_offset"].as_u64().unwrap() as usize;
        let cb = c["config_bytes"].as_u64().unwrap() as usize;
        let config = Config::parse(&configs[co..co + cb]).unwrap();
        let mut decoder = Decoder::new(&config).unwrap();
        let fresh = decoder.clone();
        let mut outputs = vec![];
        for f in c["frames"].as_array().unwrap() {
            let offset = f["offset"].as_u64().unwrap() as usize;
            let bytes = f["bytes"].as_u64().unwrap() as usize;
            let wire = &packets[offset..offset + bytes];
            let saved = decoder.clone();
            let mut reader = BitReader::new(wire);
            let a = decoder
                .decode(&mut reader)
                .unwrap_or_else(|e| panic!("case {}, frame {frames}: {e}", c["name"]));
            assert_eq!(reader.position(), f["bits"].as_u64().unwrap() as usize);
            let mut replay = saved;
            assert_eq!(replay.decode(&mut BitReader::new(wire)).unwrap(), a);
            assert_eq!(replay, decoder);
            assert_eq!(a.m1.slots.len(), config.frame_slots as usize);
            assert_eq!(a.dry.slots.len(), config.frame_slots as usize);
            assert_eq!(a.wet.slots.len(), config.frame_slots as usize);
            let bands = config.parameter_bands;
            let expected_modes = if let Some(t) = config.ttt.first() {
                (0..bands)
                    .map(|b| match t.high {
                        Some((mode, split)) if b >= split => mode,
                        _ => t.low_mode,
                    })
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            assert_eq!(a.ttt_modes, expected_modes);
            assert_eq!(
                a.residual_cutoffs,
                c["cutoffs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect::<Vec<_>>()
            );
            let mut bad = BitReader::new(&wire[..wire.len() / 2]);
            let state = decoder.clone();
            assert!(decoder.decode(&mut bad).is_err());
            assert_eq!(bad.position(), 0);
            assert_eq!(decoder, state);
            outputs.push(a);
            frames += 1;
        }
        decoder.reset();
        assert_eq!(decoder, fresh);
        for (f, expected) in c["frames"].as_array().unwrap().iter().zip(outputs) {
            let offset = f["offset"].as_u64().unwrap() as usize;
            let bytes = f["bytes"].as_u64().unwrap() as usize;
            assert_eq!(
                decoder
                    .decode(&mut BitReader::new(&packets[offset..offset + bytes]))
                    .unwrap(),
                expected
            );
        }
    }
    assert_eq!(frames, 1008);
}

#[test]
fn assembled_endpoints_and_every_slot_match_original_matrix_decimal_temporal_oracle() {
    let r = root();
    let m: Value =
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap();
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(r.join("oracle.json")).unwrap()).unwrap();
    let expected = std::fs::read(r.join("expected.bin")).unwrap();
    let configs = std::fs::read(r.join("configs.bin")).unwrap();
    let packets = std::fs::read(r.join("packets.bin")).unwrap();
    let mut maximum = 0.0f64;
    let mut scalars = 0;
    for (c, o) in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        assert_eq!(c["name"], o["name"]);
        let co = c["config_offset"].as_u64().unwrap() as usize;
        let cb = c["config_bytes"].as_u64().unwrap() as usize;
        let config = Config::parse(&configs[co..co + cb]).unwrap();
        let mut decoder = Decoder::new(&config).unwrap();
        let mut previous: Vec<Vec<f64>> = vec![vec![]; 3];
        for (f, of) in c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .zip(o["frames"].as_array().unwrap())
        {
            let offset = f["offset"].as_u64().unwrap() as usize;
            let n = f["bytes"].as_u64().unwrap() as usize;
            let a = decoder
                .decode(&mut BitReader::new(&packets[offset..offset + n]))
                .unwrap();
            let frames = [&a.m1, &a.dry, &a.wet];
            let slots: Vec<u8> = of["slots"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect();
            assert_eq!(slots, a.prepared.slots);
            for plane in 0..3 {
                let width = of["widths"][plane].as_u64().unwrap() as usize;
                let base = of["offsets"][plane].as_u64().unwrap() as usize;
                if previous[plane].is_empty() {
                    previous[plane] = vec![0.0; width];
                }
                let mut last = -1i16;
                for (ps, &slot) in slots.iter().enumerate() {
                    let endpoint = (0..width)
                        .map(|i| {
                            let at = base + (ps * width + i) * 4;
                            f32::from_le_bytes(expected[at..at + 4].try_into().unwrap()) as f64
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(frames[plane].endpoints[ps].len(), width);
                    for i in 0..width {
                        let error = (frames[plane].endpoints[ps][i] - endpoint[i]).abs();
                        maximum = maximum.max(error);
                        scalars += 1;
                        assert!(
                            error < 5e-6,
                            "endpoint error {error}, case {}, plane {plane}",
                            c["name"]
                        );
                    }
                    for t in last + 1..=i16::from(slot) {
                        let alpha = f64::from(t - last) / f64::from(i16::from(slot) - last);
                        for i in 0..width {
                            let want = (1.0 - alpha) * previous[plane][i] + alpha * endpoint[i];
                            let error = (frames[plane].slots[t as usize][i] - want).abs();
                            maximum = maximum.max(error);
                            assert!(
                                error < 5e-6,
                                "slot error {error}, case {}, plane {plane}",
                                c["name"]
                            );
                        }
                    }
                    previous[plane] = endpoint;
                    last = i16::from(slot);
                }
            }
        }
    }
    assert_eq!(scalars * 4, expected.len());
    println!(
        "assembled endpoint/slot oracle maximum error: {maximum}; endpoint scalars: {scalars}"
    );
}
#[test]
fn late_singular_matrix_failure_rolls_back_the_successfully_parsed_frame() {
    use fvid_media::owned_aac::aac_ld_mps_prepare;
    let r = root();
    let m: Value =
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap();
    let configs = std::fs::read(r.join("configs.bin")).unwrap();
    let packets = std::fs::read(r.join("packets.bin")).unwrap();
    let errors = std::fs::read(r.join("errors.bin")).unwrap();
    let e = &m["errors"][0];
    let c = &m["cases"][e["case"].as_u64().unwrap() as usize];
    let co = c["config_offset"].as_u64().unwrap() as usize;
    let cb = c["config_bytes"].as_u64().unwrap() as usize;
    let config = Config::parse(&configs[co..co + cb]).unwrap();
    let bad =
        &errors[e["offset"].as_u64().unwrap() as usize..][..e["bytes"].as_u64().unwrap() as usize];
    let mut physical = aac_ld_mps_prepare::Decoder::new(&config).unwrap();
    let mut parsed = BitReader::new(bad);
    physical.decode(&mut parsed).unwrap();
    assert_eq!(parsed.position(), bad.len() * 8);
    let mut decoder = Decoder::new(&config).unwrap();
    let f = &c["frames"][0];
    let at = f["offset"].as_u64().unwrap() as usize;
    let n = f["bytes"].as_u64().unwrap() as usize;
    decoder
        .decode(&mut BitReader::new(&packets[at..at + n]))
        .unwrap();
    let state = decoder.clone();
    let mut reader = BitReader::new(bad);
    assert!(
        decoder
            .decode(&mut reader)
            .unwrap_err()
            .to_string()
            .contains("singular LD-MPS TTT prediction coherence")
    );
    assert_eq!(reader.position(), 0);
    assert_eq!(decoder, state);
    let f = &c["frames"][1];
    let at = f["offset"].as_u64().unwrap() as usize;
    let n = f["bytes"].as_u64().unwrap() as usize;
    let mut replay = state;
    assert_eq!(
        decoder
            .decode(&mut BitReader::new(&packets[at..at + n]))
            .unwrap(),
        replay
            .decode(&mut BitReader::new(&packets[at..at + n]))
            .unwrap()
    );
    let mut cfg = config.clone();
    cfg.arbitrary_downmix = true;
    assert!(
        Decoder::new(&cfg)
            .unwrap_err()
            .to_string()
            .contains("arbitrary-downmix")
    );
}
