use fvid_media::owned_aac::{
    aac_ld_mps_config::Config, aac_ld_mps_decor::Mode, aac_ld_mps_qmf::Decoder,
    aac_sbr_qmf::Complex, bits::BitReader,
};
use serde_json::Value;
fn root(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn sample(residual: bool, frame: usize, t: usize, c: usize, q: usize) -> Complex {
    if frame == 2 {
        return Complex::default();
    }
    let denominator = if residual { 512.0 } else { 256.0 };
    Complex {
        re: (((frame * 11 + t * 7 + c * 3 + q * 5) % 29) as f64 - 14.0) / denominator,
        im: (((frame * 7 + t * 5 + c * 11 + q * 3) % 23) as f64 - 11.0) / denominator,
    }
}
fn input(
    config: &Config,
    bins: usize,
    frame: usize,
    boxes: usize,
) -> (Vec<Vec<Vec<Complex>>>, Vec<Option<Vec<Vec<Complex>>>>) {
    let slots = config.frame_slots as usize;
    let downmix = (0..slots)
        .map(|t| {
            (0..config.tree.input_channels as usize)
                .map(|c| (0..bins).map(|q| sample(false, frame, t, c, q)).collect())
                .collect()
        })
        .collect();
    let residuals = (0..boxes)
        .map(|b| {
            Some(
                (0..slots)
                    .map(|t| (0..bins).map(|q| sample(true, frame, t, b, q)).collect())
                    .collect(),
            )
        })
        .collect();
    (downmix, residuals)
}
#[test]
fn complete_qmf_chain_matches_original_stage_functions_and_checkpoint_reset_replay() {
    let base = root("aac-ld-mps-assemble");
    let m: Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let configs = std::fs::read(base.join("configs.bin")).unwrap();
    let packets = std::fs::read(base.join("packets.bin")).unwrap();
    let r = root("aac-ld-mps-qmf128");
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap();
    let expected = fvid_media::owned_matroska_zlib::decode(
        &std::fs::read(r.join("expected.bin.zlib")).unwrap(),
        oracle["expected_bytes"].as_u64().unwrap() as usize,
    )
    .unwrap();
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 112);
    let mut maximum = 0.0f64;
    let mut scalars = 0;
    let mut tail_nonzero = 0;
    for o in oracle["cases"].as_array().unwrap() {
        let c = &m["cases"][o["base_case"].as_u64().unwrap() as usize];
        let at = c["config_offset"].as_u64().unwrap() as usize;
        let n = c["config_bytes"].as_u64().unwrap() as usize;
        let config = Config::parse(&configs[at..at + n]).unwrap();
        let mode = if o["mode"] == 0 {
            Mode::Real
        } else {
            Mode::Complex
        };
        let bins = o["bins"].as_u64().unwrap() as usize;
        let mut decoder = Decoder::new(&config, mode).unwrap();
        let fresh = decoder.clone();
        let mut outputs = vec![];
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
            let (downmix, residuals) =
                input(&config, bins, fi, c["cutoffs"].as_array().unwrap().len());
            let state = decoder.clone();
            let mut reader = BitReader::new(wire);
            let output = decoder.decode(&mut reader, &downmix, &residuals).unwrap();
            assert_eq!(reader.position(), f["bits"].as_u64().unwrap() as usize);
            let mut replay = state;
            assert_eq!(
                replay
                    .decode(&mut BitReader::new(wire), &downmix, &residuals)
                    .unwrap(),
                output
            );
            assert_eq!(replay, decoder);
            let mut offset = of["offset"].as_u64().unwrap() as usize;
            for (element, z) in output
                .dry
                .iter()
                .chain(&output.wet)
                .flatten()
                .flatten()
                .enumerate()
            {
                for value in [z.re, z.im] {
                    let reference =
                        f32::from_le_bytes(expected[offset..offset + 4].try_into().unwrap()) as f64;
                    offset += 4;
                    scalars += 1;
                    let error = (value - reference).abs();
                    maximum = maximum.max(error);
                    assert!(
                        error < 2e-6,
                        "error {error}: {value} vs {reference}, case {} frame {fi} element{element} bin{}",
                        o["name"],
                        element % bins
                    );
                }
            }
            if fi == 2
                && output
                    .wet
                    .iter()
                    .flatten()
                    .flatten()
                    .any(|z| z.re.abs() + z.im.abs() > 1e-10)
            {
                tail_nonzero += 1;
            }
            outputs.push(output);
        }
        decoder.reset();
        assert_eq!(decoder, fresh);
        for (fi, (f, want)) in c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .zip(outputs)
            .enumerate()
        {
            let at = f["offset"].as_u64().unwrap() as usize;
            let n = f["bytes"].as_u64().unwrap() as usize;
            let (downmix, residuals) =
                input(&config, bins, fi, c["cutoffs"].as_array().unwrap().len());
            assert_eq!(
                decoder
                    .decode(
                        &mut BitReader::new(&packets[at..at + n]),
                        &downmix,
                        &residuals
                    )
                    .unwrap(),
                want
            );
        }
    }
    assert_eq!(scalars * 4, expected.len());
    assert!(tail_nonzero > 100);
    println!(
        "128-bin QMF chain maximum original-stage error: {maximum}; scalars: {scalars}; nonzero tail sequences: {tail_nonzero}"
    );
}

#[test]
fn upper_rows_retain_their_source_delay_without_allpass_or_ducking() {
    use fvid_media::owned_aac::aac_ld_mps_decor::Decorrelator;
    let mut decoder = Decorrelator::new(128, 0, 0, Mode::Complex).unwrap();
    let mut input = vec![vec![Complex::default(); 128]; 5];
    input[0][71] = Complex { re: 0.5, im: -0.25 };
    input[1][71] = Complex {
        re: 0.75,
        im: -0.125,
    };
    input[0][127] = Complex { re: 0.25, im: -0.5 };
    input[1][127] = Complex {
        re: -0.125,
        im: 0.75,
    };
    input[0][70] = Complex { re: 0.01, im: 0.03 };
    let output = decoder.process(&input).unwrap();
    for bin in [71, 127] {
        assert_eq!(output[0][bin], Complex::default());
        for t in 1..5 {
            assert_eq!(output[t][bin], input[t - 1][bin]);
        }
    }
    assert_ne!(output[1][70], input[0][70]);
    let saved = decoder.clone();
    let mut bad = input.clone();
    bad[4][127].im = f64::NAN;
    assert!(decoder.process(&bad).is_err());
    assert_eq!(decoder, saved);
    decoder.reset();
    let again = decoder.process(&input).unwrap();
    assert_eq!(again, output);
}
