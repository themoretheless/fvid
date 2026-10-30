use fvid_media::owned_aac::{
    aac_ld_mps_config::Config,
    aac_ld_mps_decor::Mode,
    aac_ld_mps_pcm::{Decoder, native_order},
    aac_sbr_qmf::Complex,
    bits::BitReader,
};
use serde_json::Value;
fn root(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn pcm(frame: usize, channels: usize, n: usize) -> Vec<Vec<f64>> {
    (0..channels)
        .map(|c| {
            (0..n)
                .map(|i| {
                    if frame == 2 {
                        0.0
                    } else {
                        (((frame * 11 + i * 7 + c * 3) % 29) as f64 - 14.0) / 1024.0
                    }
                })
                .collect()
        })
        .collect()
}
fn residuals(
    frame: usize,
    boxes: usize,
    slots: usize,
    bins: usize,
) -> Vec<Option<Vec<Vec<Complex>>>> {
    (0..boxes)
        .map(|c| {
            Some(
                (0..slots)
                    .map(|t| {
                        (0..bins)
                            .map(|q| {
                                if frame == 2 {
                                    Complex::default()
                                } else {
                                    Complex {
                                        re: (((frame * 11 + t * 7 + c * 3 + q * 5) % 29) as f64
                                            - 14.0)
                                            / 512.0,
                                        im: (((frame * 7 + t * 5 + c * 11 + q * 3) % 23) as f64
                                            - 11.0)
                                            / 512.0,
                                    }
                                }
                            })
                            .collect()
                    })
                    .collect(),
            )
        })
        .collect()
}
#[test]
fn pcm_chain_matches_reference_analysis_synthesis_order_and_all_fixed_gain_grids() {
    let b = root("aac-ld-mps-assemble");
    let m: Value =
        serde_json::from_slice(&std::fs::read(b.join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(b.join("packets.bin")).unwrap();
    let r = root("aac-ld-mps-pcm");
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap();
    let expected = std::fs::read(r.join("expected.bin")).unwrap();
    let original = std::fs::read(r.join("original.bin")).unwrap();
    let mut maximum = 0.0f64;
    let mut original_maximum = 0.0f64;
    let mut scalars = 0;
    let mut gain_seen = [
        std::collections::BTreeSet::new(),
        std::collections::BTreeSet::new(),
        std::collections::BTreeSet::new(),
    ];
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 224);
    for o in oracle["cases"].as_array().unwrap() {
        let c = &m["cases"][o["base_case"].as_u64().unwrap() as usize];
        let config = Config::parse(&hex(o["config"].as_str().unwrap())).unwrap();
        for i in 0..3 {
            gain_seen[i].insert(o["gain_indices"][i].as_u64().unwrap());
        }
        assert_eq!(
            config.downmix_gain as u64,
            o["gain_indices"][0].as_u64().unwrap()
        );
        if config.tree.code != 7 {
            assert_eq!(
                config.surround_gain.unwrap() as u64,
                o["gain_indices"][1].as_u64().unwrap()
            );
            assert_eq!(
                config.lfe_gain.unwrap() as u64,
                o["gain_indices"][2].as_u64().unwrap()
            );
        }
        let mode = if o["mode"] == 0 {
            Mode::Real
        } else {
            Mode::Complex
        };
        let bins = o["bins"].as_u64().unwrap() as usize;
        let mut decoder = Decoder::new(&config, mode).unwrap();
        let fresh = decoder.clone();
        let mut saved_outputs = vec![];
        for (fi, (f, of)) in c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .zip(o["frames"].as_array().unwrap())
            .enumerate()
        {
            let at = f["offset"].as_u64().unwrap() as usize;
            let n = f["bytes"].as_u64().unwrap() as usize;
            let wire = &packets[at..at + n];
            let x = pcm(
                fi,
                config.tree.input_channels as usize,
                config.frame_slots as usize * bins,
            );
            let res = residuals(
                fi,
                c["cutoffs"].as_array().unwrap().len(),
                config.frame_slots as usize,
                bins,
            );
            let state = decoder.clone();
            let mut reader = BitReader::new(wire);
            let output = decoder.decode(&mut reader, &x, &res).unwrap();
            assert_eq!(reader.position(), f["bits"].as_u64().unwrap() as usize);
            assert_eq!(output.channels, config.tree.output_channels as usize);
            assert_eq!(output.sample_rate, config.sample_rate);
            assert_eq!(
                output.samples.len(),
                config.frame_slots as usize * bins * output.channels
            );
            let mut replay = state;
            assert_eq!(
                replay.decode(&mut BitReader::new(wire), &x, &res).unwrap(),
                output
            );
            assert_eq!(replay, decoder);
            let offset = of["offset"].as_u64().unwrap() as usize;
            for (i, &actual) in output.samples.iter().enumerate() {
                let at = offset + i * 4;
                let want = f32::from_le_bytes(expected[at..at + 4].try_into().unwrap()) as f64;
                let old = f32::from_le_bytes(original[at..at + 4].try_into().unwrap()) as f64;
                let error = (actual - want).abs();
                let old_error = (actual - old).abs();
                maximum = maximum.max(error);
                original_maximum = original_maximum.max(old_error);
                scalars += 1;
                assert!(
                    error < 5e-6,
                    "{actual} vs {want}, error {error}, case {} frame {fi}",
                    o["name"]
                );
                assert!(
                    old_error < 5e-5,
                    "original PI control error {old_error}, case {}",
                    o["name"]
                );
            }
            saved_outputs.push(output);
        }
        decoder.reset();
        assert_eq!(decoder, fresh);
        for (fi, (f, want)) in c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .zip(saved_outputs)
            .enumerate()
        {
            let at = f["offset"].as_u64().unwrap() as usize;
            let n = f["bytes"].as_u64().unwrap() as usize;
            let x = pcm(
                fi,
                config.tree.input_channels as usize,
                config.frame_slots as usize * bins,
            );
            let res = residuals(
                fi,
                c["cutoffs"].as_array().unwrap().len(),
                config.frame_slots as usize,
                bins,
            );
            assert_eq!(
                decoder
                    .decode(&mut BitReader::new(&packets[at..at + n]), &x, &res)
                    .unwrap(),
                want
            );
        }
    }
    assert_eq!(scalars * 4, expected.len());
    assert_eq!(
        gain_seen.iter().map(|s| s.len()).collect::<Vec<_>>(),
        [8, 5, 5]
    );
    println!(
        "PCM chain max precise-PI error: {maximum}; original-PI error: {original_maximum}; samples: {scalars}"
    );
    assert_eq!(native_order(0).unwrap(), [0, 1, 2, 3, 4, 5]);
    assert_eq!(native_order(1).unwrap(), [0, 2, 4, 5, 1, 3]);
    assert_eq!(native_order(4).unwrap(), [0, 3, 6, 7, 2, 5, 1, 4]);
}
#[test]
fn late_active_temporal_tool_refusal_and_bad_pcm_roll_back_the_whole_pcm_chain() {
    let r = root("aac-ld-mps-frame");
    let m: Value =
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap();
    let configs = std::fs::read(r.join("configs.bin")).unwrap();
    let packets = std::fs::read(r.join("packets.bin")).unwrap();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "tree0-res1-v4")
        .unwrap();
    let at = c["config_offset"].as_u64().unwrap() as usize;
    let n = c["config_bytes"].as_u64().unwrap() as usize;
    let config = Config::parse(&configs[at..at + n]).unwrap();
    let mut decoder = Decoder::new(&config, Mode::Complex).unwrap();
    let x = pcm(0, 1, config.frame_slots as usize * 64);
    let res = vec![None; 5];
    let frame = |i: usize| {
        let f = &c["frames"][i];
        let at = f["offset"].as_u64().unwrap() as usize;
        let n = f["bytes"].as_u64().unwrap() as usize;
        &packets[at..at + n]
    };
    decoder
        .decode(&mut BitReader::new(frame(0)), &x, &res)
        .unwrap();
    let saved = decoder.clone();
    let mut reader = BitReader::new(frame(1));
    assert!(
        decoder
            .decode(&mut reader, &x, &res)
            .unwrap_err()
            .to_string()
            .contains("active temporal PCM shaping")
    );
    assert_eq!(reader.position(), 0);
    assert_eq!(decoder, saved);
    let mut bad = x.clone();
    bad[0][0] = f64::NAN;
    assert!(
        decoder
            .decode(&mut BitReader::new(frame(2)), &bad, &res)
            .is_err()
    );
    assert_eq!(decoder, saved);
    let mut replay = saved;
    assert_eq!(
        decoder
            .decode(&mut BitReader::new(frame(2)), &x, &res)
            .unwrap(),
        replay
            .decode(&mut BitReader::new(frame(2)), &x, &res)
            .unwrap()
    );
}
