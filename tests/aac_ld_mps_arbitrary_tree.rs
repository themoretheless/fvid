use std::io::Cursor;
use fvid_media::owned_aac::{aac_ld_mps_arbitrary_tree::Decoder, aac_sbr_qmf::Complex};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-arbitrary-tree")
}
fn json(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn floats(b: &[u8]) -> Vec<f64> {
    b.chunks_exact(4)
        .map(|v| f64::from(f32::from_le_bytes(v.try_into().unwrap())))
        .collect()
}
#[test]
fn post_shape_routing_matches_unmodified_iso_c_with_history_reset_and_rollback() {
    let m = json("component.json");
    let oracle = json("component-oracle.json");
    let input = std::fs::read(root().join("component-input.f32le")).unwrap();
    let expected = std::fs::read(root().join("component-expected.f32le")).unwrap();
    let mut count = 0;
    let mut maximum = 0.0f64;
    assert_eq!(m["cases"].as_array().unwrap().len(), 42);
    for (case, reference) in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        let nodes: Vec<Vec<bool>> = serde_json::from_value(case["nodes"].clone()).unwrap();
        let cutoffs: Vec<u8> = serde_json::from_value(case["cutoffs"].clone()).unwrap();
        let positions: Vec<u8> = serde_json::from_value(case["positions"].clone()).unwrap();
        let map: Vec<u8> = serde_json::from_value(case["map"].clone()).unwrap();
        let fixed: Vec<f64> = serde_json::from_value(case["fixed"].clone()).unwrap();
        let mut d = Decoder::new(&nodes, &cutoffs, &positions, n(&case["bands"]) as u8).unwrap();
        let initial = d.clone();
        for (block, gold) in case["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .zip(reference["blocks"].as_array().unwrap())
        {
            let slots = n(&block["slots"]);
            let channels = n(&case["inputs"]);
            let bank = n(&case["bank"]);
            let at = n(&block["offset"]);
            let values = floats(&input[at..at + slots * channels * bank * 8]);
            let mut source = vec![vec![vec![Complex::default(); bank]; channels]; slots];
            for (t, row) in source.iter_mut().enumerate() {
                for (c, bins) in row.iter_mut().enumerate() {
                    for (q, z) in bins.iter_mut().enumerate() {
                        let i = ((t * channels + c) * bank + q) * 2;
                        *z = Complex {
                            re: values[i],
                            im: values[i + 1],
                        };
                    }
                }
            }
            let endpoints: Vec<u8> = serde_json::from_value(block["endpoints"].clone()).unwrap();
            let cld: Vec<Vec<Vec<f64>>> = serde_json::from_value(block["cld"].clone()).unwrap();
            let checkpoint = d.clone();
            let mut bad = source.clone();
            bad[slots - 1][channels - 1][bank - 1].re = f64::NAN;
            assert!(d.process(&bad, &endpoints, &cld, &map, &fixed).is_err());
            assert_eq!(d, checkpoint);
            let got = d.process(&source, &endpoints, &cld, &map, &fixed).unwrap();
            let mut replay = checkpoint;
            assert_eq!(
                got,
                replay
                    .process(&source, &endpoints, &cld, &map, &fixed)
                    .unwrap()
            );
            assert_eq!(d, replay);
            let at = n(&gold["offset"]);
            let want = floats(&expected[at..at + slots * n(&case["outputs"]) * bank * 8]);
            for (a, b) in got
                .iter()
                .flatten()
                .flatten()
                .flat_map(|z| [z.re, z.im])
                .zip(want)
            {
                let error = (a - b).abs();
                maximum = maximum.max(error);
                assert!(
                    error <= 2e-7 * (1.0 + b.abs()),
                    "tree{} bank{bank} band{}: {a} != {b}",
                    case["tree"],
                    case["bands"]
                );
                count += 1;
            }
        }
        d.reset();
        assert_eq!(d, initial);
    }
    eprintln!("Arbitrary-tree original C: {count} scalar outputs, max error {maximum}");
}
#[test]
fn extra_ott_movie_accepts_complete_valid_parameters_and_matches_analytic_gain_composition() {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::{Config, ExtensionValue},
        aac_ld_mps_frame,
        bits::BitReader,
    };
    let m = json("manifest.json");
    let c = &m["cases"][0];
    let hex = |v: &Value| {
        let s = v.as_str().unwrap();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>()
    };
    let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
    cfg.validate_low_delay_profile().unwrap();
    let ExtensionValue::ArbitraryTree {
        nodes,
        ott,
        positions,
    } = &cfg.extensions[0].value
    else {
        panic!("missing additional OTT")
    };
    assert_eq!(nodes, &[vec![true, false, false], vec![false]]);
    assert_eq!(ott.len(), 1);
    assert_eq!(positions, &[0, 1, 2]);
    let mut syntax = aac_ld_mps_frame::Decoder::new(&cfg).unwrap();
    for wire in c["reference"].as_array().unwrap() {
        let data = hex(wire);
        let mut b = BitReader::new(&data);
        let f = syntax.decode(&mut b).unwrap();
        assert_eq!(b.remaining(), 0);
        assert_eq!(f.extensions[0].parameters.len(), 1);
        assert_eq!(f.extensions[0].parameters[0].values.len(), 2);
    }
    let expected = waveform(c, fvid_media::owned_aac::aac_ld_mps_decor::Mode::Complex);
    for mkv in [false, true] {
        let mut out = Vec::new();
        let bytes = read_case(c, if mkv { "file" } else { "video" });
        if mkv {
            fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                std::io::Cursor::new(bytes),
                &mut out,
                None,
                &Default::default(),
            )
            .unwrap();
        } else {
            fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut out).unwrap();
        }
        assert_eq!(out.len(), 768 * 3 * 4);
        near(&out, &expected);
    }
}
#[test]
fn malformed_topology_and_unqualified_domains_do_not_construct() {
    assert!(Decoder::new(&[vec![true, false]], &[4], &[0, 1], 4).is_err());
    assert!(Decoder::new(&[vec![false, false]], &[], &[0], 4).is_err());
    assert!(Decoder::new(&[vec![true, false, false]], &[4], &[0, 0], 4).is_err());
    assert!(Decoder::new(&[vec![true, false, false]], &[3], &[0, 1], 4)
        .unwrap_err()
        .to_string()
        .contains("partial-band"));
    assert!(Decoder::new(&[vec![false]], &[], &[2], 4)
        .unwrap_err()
        .to_string()
        .contains("sparse"));
}

#[test]
fn balanced_split_has_analytic_energy_and_unity_initialized_gain_ramps() {
    let mut d = Decoder::new(
        &[vec![true, true, false, false, true, false, false]],
        &[4, 4, 4],
        &[3, 1, 0, 2],
        4,
    )
    .unwrap();
    let input = vec![vec![vec![Complex { re: 1.0, im: 0.0 }; 4]]; 5];
    let levels = vec![vec![vec![0.0; 4]; 3]];
    let first = d
        .process(&input, &[4], &levels, &[0, 1, 2, 3], &[1.0])
        .unwrap();
    for (t, row) in first.iter().enumerate() {
        let expected = 1.0 - 0.5 * (t + 1) as f64 / 5.0;
        for z in row.iter().flatten() {
            assert!((z.re - expected).abs() < 1e-14);
            assert_eq!(z.im, 0.0);
        }
    }
    let steady = d
        .process(&input, &[4], &levels, &[0, 1, 2, 3], &[1.0])
        .unwrap();
    for row in steady {
        for q in 0..4 {
            let energy = row.iter().map(|c| c[q].re * c[q].re).sum::<f64>();
            assert!((energy - 1.0).abs() < 1e-14);
        }
    }
    d.reset();
    assert_eq!(
        d.process(&input, &[4], &levels, &[0, 1, 2, 3], &[1.0])
            .unwrap(),
        first
    );
}

fn read_case(c: &Value, key: &str) -> Vec<u8> {
    std::fs::read(root().join(c[key].as_str().unwrap())).unwrap()
}
fn hex(v: &Value) -> Vec<u8> {
    let s = v.as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn near(a: &[u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    let maximum = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(a, b)| {
            (f32::from_le_bytes(a.try_into().unwrap()) - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
        })
        .fold(0.0f32, f32::max);
    assert!(maximum < 1e-7, "PCM max error {maximum}");
}
fn waveform(c: &Value, mode: fvid_media::owned_aac::aac_ld_mps_decor::Mode) -> Vec<u8> {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::Config,
        aac_ld_mps_qmf,
        aac_ld_qmf::{Analysis, Synthesis},
        bits::BitReader,
    };
    let cfg = Config::parse(&hex(&c["base_ssc"])).unwrap();
    let mut spatial = aac_ld_mps_qmf::Decoder::new(&cfg, mode).unwrap();
    let mut analysis = Analysis::<32>::new().unwrap();
    let mut synthesis = (0..3)
        .map(|_| Synthesis::<32>::new().unwrap())
        .collect::<Vec<_>>();
    let core = floats(&read_case(c, "core_gold"));
    let mut previous = [[1.0f64; 23]; 2];
    let mut result = Vec::new();
    for frame in 0..3 {
        let qmf = analysis
            .process(&core[frame * 256..(frame + 1) * 256])
            .unwrap();
        let input = qmf
            .into_iter()
            .map(|s| vec![s.to_vec()])
            .collect::<Vec<_>>();
        let bytes = hex(&c["base_reference"][frame]);
        let base = spatial
            .decode(&mut BitReader::new(&bytes), &input, &[None])
            .unwrap();
        let mut routed = vec![vec![vec![Complex::default(); 32]; 3]; 8];
        for t in 0..8 {
            for q in 0..32 {
                let band = usize::from(base.prepared.qmf_parameter_bands[q]);
                let ratio = 10.0f64.powf(c["extra_cld"][frame][band].as_f64().unwrap() / 10.0);
                let weights = [(ratio / (1.0 + ratio)).sqrt(), (1.0 / (1.0 + ratio)).sqrt()];
                for ch in 0..2 {
                    let weight = if t < 4 {
                        previous[ch][band]
                            + (weights[ch] - previous[ch][band]) * (t + 1) as f64 / 4.0
                    } else {
                        weights[ch]
                    };
                    let z = Complex {
                        re: base.dry[t][0][q].re + base.wet[t][0][q].re,
                        im: base.dry[t][0][q].im + base.wet[t][0][q].im,
                    };
                    routed[t][ch][q] = Complex {
                        re: z.re * weight,
                        im: z.im * weight,
                    };
                }
                routed[t][2][q] = Complex {
                    re: base.dry[t][1][q].re + base.wet[t][1][q].re,
                    im: base.dry[t][1][q].im + base.wet[t][1][q].im,
                };
            }
        }
        for band in 0..23 {
            let ratio = 10.0f64.powf(c["extra_cld"][frame][band].as_f64().unwrap() / 10.0);
            previous[0][band] = (ratio / (1.0 + ratio)).sqrt();
            previous[1][band] = (1.0 / (1.0 + ratio)).sqrt();
        }
        let planar = (0..3)
            .map(|ch| {
                synthesis[ch]
                    .process(
                        &routed
                            .iter()
                            .map(|s| std::array::from_fn(|q| s[ch][q]))
                            .collect::<Vec<[Complex; 32]>>(),
                    )
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for t in 0..256 {
            for row in &planar {
                result.extend_from_slice(&(row[t] as f32).to_le_bytes());
            }
        }
    }
    result
}

fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        let stamp = reader.decoder_stamp(&packet);
        if let Some(f) = reader
            .decode_source_packet(decoder.as_mut(), &packet, stamp)
            .unwrap()
        {
            if let Some(p) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                out.extend(p.data);
            }
        }
    }
    while let Some(f) = decoder.finish_packet().unwrap() {
        if let Some(p) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
            out.extend(p.data);
        }
    }
    out
}
#[test]
fn downscaled_spatial_playback_seek_and_rewind_keep_source_windows() {
    use fvid::audio::AudioStream;
    for c in json("manifest.json")["cases"].as_array().unwrap() {
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read_case(c, "video"), &mut full).unwrap();
        for mkv in [false, true] {
            let mut reader: Box<dyn AudioStream> = if mkv {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(read_case(c, "file")),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(read_case(c, "video")),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            assert_eq!(play(reader.as_mut()), full);
            for sample in [1, 255, 256, 257, 384, 511, 512, 513, 768] {
                reader.rewind();
                let ticks = (sample as u128 * u128::from(reader.timescale())
                    / u128::from(reader.sample_rate())) as i64;
                let landed = reader.seek_to(ticks);
                let begin = (u128::try_from(landed).unwrap() * u128::from(reader.sample_rate()))
                    .div_ceil(u128::from(reader.timescale())) as usize
                    * 3
                    * 4;
                assert_eq!(
                    play(reader.as_mut()),
                    full[begin.min(full.len())..],
                    "{} seek{sample} mkv{mkv}",
                    c["name"]
                );
            }
            reader.rewind();
            assert_eq!(play(reader.as_mut()), full);
        }
    }
}

#[test]
fn native_extra_channels_preserve_clocks_checkpoints_and_failed_suffix_retry() {
    use fvid_media::owned_aac::{
        aac_ld_mps_decor::Mode, AacDecodeOptions, config::AudioSpecificConfig, NativeAacDecoder,
    };
    let m = json("manifest.json");
    let c = &m["cases"][0];
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let asc = hex(&c["asc"]);
    assert_eq!(
        AudioSpecificConfig::parse(&asc).unwrap().output_channels(),
        3
    );
    for mode in [Mode::Real, Mode::Complex] {
        let mut d = NativeAacDecoder::new_with_options(
            &asc,
            AacDecodeOptions {
                output_sample_rate: Some(24000),
                spatial_decorrelation: mode,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(d.channels(), 3);
        assert_eq!(d.channel_mask(), 0);
        assert!(d.channel_positions().unwrap().is_none());
        let mut actual = Vec::new();
        let mut control = NativeAacDecoder::new_with_options(&asc,AacDecodeOptions {output_sample_rate:Some(24000),spatial_decorrelation:mode,..Default::default()}).unwrap();
        for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = n(&r["offset"]);
            let packet = &blob[at..at + n(&r["bytes"])];
            let pts = (i * 256) as i64;
            let before = d.checkpoint();
            let footprint = d.retained_payload_bytes().unwrap();
            assert!(d
                .decode_timed_in_clock(&packet[..packet.len() - 1], pts, 256, 24000)
                .is_err());
            assert_eq!(d.retained_payload_bytes().unwrap(), footprint);
            let want = control
                .decode_timed_in_clock(packet, pts, 256, 24000)
                .unwrap();
            let got = d.decode_timed_in_clock(packet, pts, 256, 24000).unwrap();
            assert_eq!(got, want);
            d.restore(&before).unwrap();
            assert_eq!(
                d.decode_timed_in_clock(packet, pts, 256, 24000).unwrap(),
                want
            );
            assert!(
                d.retained_payload_bytes_with_checkpoint(Some(&before))
                    .unwrap()
                    > d.retained_payload_bytes().unwrap()
            );
            if let Some(f) = got {
                assert_eq!(f.samples.len(), 256*3);
                assert_eq!(f.pts, pts);
                assert_eq!(f.duration, 256);
                actual.extend(f.samples.iter().flat_map(|v| v.to_le_bytes()));
            }
        }
        while let Some(f) = d.finish().unwrap() {
            actual.extend(f.samples.iter().flat_map(|v| v.to_le_bytes()));
        }
        near(&actual, &waveform(c, mode));
        d.reset();
        assert_eq!(d.channels(), 3);
    }
}
