use fvid_media::owned_aac::{
    AacDecodeOptions, NativeAacDecoder, aac_ld_mps_config::Config, aac_ld_mps_decor::Mode,
    aac_ld_mps_pcm, bits::BitReader,
};

fn fixture(corpus: &str, name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(corpus)
            .join(name),
    )
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn both_native_decorrelators_match_the_explicit_spatial_renderer() {
    let mut differences = 0;
    for tree in 0..8 {
        let read = |name: &str| fixture("aac-eld-mps-core", &format!("tree{tree}{name}"));
        let asc = read(".spatial.asc");
        let cfg = Config::parse(&read(".ssc")).unwrap();
        let inputs = usize::from(cfg.tree.input_channels);
        let outputs = usize::from(cfg.tree.output_channels);
        let core: Vec<f64> = read(".pcm")
            .chunks_exact(4)
            .map(|v| f64::from(f32::from_le_bytes(v.try_into().unwrap())))
            .collect();
        let mut modes = Vec::new();
        for mode in [Mode::Real, Mode::Complex] {
            let mut native = NativeAacDecoder::new_with_options(
                &asc,
                AacDecodeOptions {
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut frontend = fvid::codec::aac_native::NativeAacDecoder::new_with_options(
                &asc,
                fvid::codec::aac_native::AacDecodeOptions {
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut spatial = aac_ld_mps_pcm::Decoder::new(&cfg, mode).unwrap();
            let mut actual = Vec::new();
            for i in 0..3 {
                let planar: Vec<Vec<_>> = (0..inputs)
                    .map(|ch| {
                        (0..512)
                            .map(|t| core[((i * 512 + t) * inputs) + ch])
                            .collect()
                    })
                    .collect();
                let expected = spatial
                    .decode(
                        &mut BitReader::new(&read(&format!("-{i}.spatial"))),
                        &planar,
                        &vec![None; cfg.tree.ott_lfe.len() + usize::from(cfg.tree.ttt_boxes)],
                    )
                    .unwrap();
                let packet = read(&format!("-{i}.au"));
                let frame = native
                    .decode_timed(&packet, i as i64 * 512, 512)
                    .unwrap()
                    .unwrap();
                let front = frontend
                    .decode_timed(&packet, i as i64 * 512, 512)
                    .unwrap()
                    .unwrap();
                assert_eq!(front.samples, frame.samples);
                assert_eq!((front.pts, front.duration), (frame.pts, frame.duration));
                assert_eq!((frame.pts, frame.duration), (i as i64 * 512, 512));
                assert_eq!(frame.samples.len(), expected.samples.len());
                for (a, row) in frame
                    .samples
                    .chunks_exact(outputs)
                    .zip(expected.samples.chunks_exact(outputs))
                {
                    for (ch, &v) in a.iter().enumerate() {
                        let index = if matches!(tree, 4 | 6) {
                            [0, 1, 2, 3, 6, 7, 4, 5][ch]
                        } else {
                            ch
                        };
                        assert!(
                            (f64::from(v) - row[index]).abs() < 1e-7,
                            "tree{tree} {mode:?}"
                        );
                    }
                }
                actual.extend(frame.samples);
            }
            assert!(native.finish().unwrap().is_none());
            modes.push(actual);
        }
        differences += usize::from(modes[0] != modes[1]);
        let mut default = NativeAacDecoder::new(&asc).unwrap();
        let mut ordinary = Vec::new();
        for i in 0..3 {
            ordinary.extend(default.decode(&read(&format!("-{i}.au"))).unwrap());
        }
        assert_eq!(ordinary, modes[1]);
    }
    assert!(
        differences > 0,
        "the mode selection must affect actual decorrelation"
    );
}

#[test]
fn mode_survives_reset_and_checkpoint_with_output_rate_negotiation() {
    let m: serde_json::Value =
        serde_json::from_slice(&fixture("aac-alignment", "manifest.json")).unwrap();
    let data = fixture("aac-alignment", "packets.bin");
    for c in m["programmes"].as_array().unwrap() {
        let v = &c["variants"][1];
        let asc = hex(v["asc"].as_str().unwrap());
        let rate = v["container_rate"].as_u64().unwrap() as u32;
        let n = v["container_frame_samples"].as_u64().unwrap();
        for mode in [Mode::Real, Mode::Complex] {
            let options = AacDecodeOptions {
                output_sample_rate: Some(rate),
                spatial_decorrelation: mode,
                ..Default::default()
            };
            let mut decoder = NativeAacDecoder::new_with_options(&asc, options).unwrap();
            let mut legacy = NativeAacDecoder::new_with_output_rate(&asc, rate).unwrap();
            assert_eq!(decoder.sample_rate(), rate);
            let mut expected = Vec::new();
            for (i, f) in v["frames"].as_array().unwrap().iter().enumerate() {
                let at = f["offset"].as_u64().unwrap() as usize;
                let packet = &data[at..at + f["bytes"].as_u64().unwrap() as usize];
                let saved = decoder.checkpoint();
                let a = decoder
                    .decode_timed(packet, (i as u64 * n) as i64, n)
                    .unwrap();
                if mode == Mode::Complex {
                    assert_eq!(
                        legacy
                            .decode_timed(packet, (i as u64 * n) as i64, n)
                            .unwrap(),
                        a
                    );
                }
                decoder.restore(&saved).unwrap();
                assert_eq!(
                    decoder
                        .decode_timed(packet, (i as u64 * n) as i64, n)
                        .unwrap(),
                    a
                );
                expected.push(a);
            }
            assert!(decoder.finish().unwrap().is_none());
            decoder.reset();
            for (i, f) in v["frames"].as_array().unwrap().iter().enumerate() {
                let at = f["offset"].as_u64().unwrap() as usize;
                assert_eq!(
                    decoder
                        .decode_timed(
                            &data[at..at + f["bytes"].as_u64().unwrap() as usize],
                            (i as u64 * n) as i64,
                            n
                        )
                        .unwrap(),
                    expected[i]
                );
            }
        }
    }
}

#[test]
fn cross_mode_checkpoint_is_rejected_before_mutating_either_decoder() {
    let asc = fixture("aac-eld-mps-core", "tree0.spatial.asc");
    let packet = fixture("aac-eld-mps-core", "tree0-0.au");
    for (a, b) in [(Mode::Real, Mode::Complex), (Mode::Complex, Mode::Real)] {
        let make = |mode| {
            NativeAacDecoder::new_with_options(
                &asc,
                AacDecodeOptions {
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let donor = make(a);
        let mut receiver = make(b);
        let before = receiver.retained_payload_bytes().unwrap();
        assert!(
            receiver
                .restore(&donor.checkpoint())
                .unwrap_err()
                .to_string()
                .contains("LD-MPS checkpoint configuration")
        );
        assert_eq!(receiver.retained_payload_bytes().unwrap(), before);
        assert_eq!(
            receiver.decode(&packet).unwrap(),
            make(b).decode(&packet).unwrap()
        );
    }
}
